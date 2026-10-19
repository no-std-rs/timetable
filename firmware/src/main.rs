#![no_std]
#![no_main]
extern crate alloc;
mod http_request;
mod http_session;
mod rtc;
mod tls_cipher;
mod tls_memory;
mod transport;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{Runner, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull, RtcPin, RtcPinWithResistors},
    i2c::master::{Config as I2cConfig, I2c},
    interrupt::software::SoftwareInterruptControl,
    rng::Rng,
    rtc_cntl::{
        Rtc,
        sleep::{RtcioWakeupSource, TimerWakeupSource, WakeupLevel},
        wakeup_cause,
    },
    spi::master::{Config as SpiConfig, Spi},
    system::SleepSource,
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{
    AuthenticationMethod, Config as WifiConfig, Interface, PowerSaveMode, WifiController,
    WifiError, sta::StationConfig,
};
use esp_storage::FlashStorage;
use log::{info, warn};
use static_cell::{ConstStaticCell, StaticCell};
use timetable_display::{
    App, Config, catalog::Catalog, config, feed::Builder, model::Status, render, ui,
};
use timetable_firmware::{battery, panel, power, settings};
use transport::{Error, Http};

struct State {
    config: Config,
    app: App,
    revision: u32,
    network_busy: bool,
    completed_delay: Option<u32>,
    powering_down: bool,
}
type Shared = Mutex<CriticalSectionRawMutex, State>;
static STATE: StaticCell<Shared> = StaticCell::new();
static NET: StaticCell<StackResources<4>> = StaticCell::new();
static SETTINGS: StaticCell<[u8; config::IMAGE_SIZE]> = StaticCell::new();
const _: () = assert!(panel::TILE_BYTES <= config::IMAGE_SIZE);
static FEED: StaticCell<Builder> = StaticCell::new();
static QUERY_CONFIG: ConstStaticCell<Config> = ConstStaticCell::new(Config::empty());
static WORKSPACE: ConstStaticCell<gtfs::zip::Workspace> =
    ConstStaticCell::new(gtfs::zip::Workspace::new());
static REFRESH: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static POWER_DOWN: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static RADIO_OFF: Signal<CriticalSectionRawMutex, ()> = Signal::new();
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut RETAINED: [u8; battery::RETAINED_BYTES] = [0; battery::RETAINED_BYTES];
esp_bootloader_esp_idf::esp_app_desc!();

fn log_heap() {
    let stats = esp_alloc::HEAP.stats();
    info!(
        "Heap usage: {} B current / {} B peak / {} B capacity",
        stats.current_usage, stats.max_usage, stats.size
    );
}

#[inline(never)]
fn has_settings_partition(flash: &mut FlashStorage<'_>) -> bool {
    use esp_bootloader_esp_idf::partitions::{PARTITION_TABLE_MAX_LEN, read_partition_table};
    let mut table = [0; PARTITION_TABLE_MAX_LEN];
    let Ok(table) = read_partition_table(flash, &mut table) else {
        return false;
    };
    table.iter().any(|p| {
        p.label().starts_with(b"timetable\0")
            && p.offset() == config::FLASH_OFFSET
            && p.len() == 0x10000
            && p.raw_type() == 1
            && p.raw_subtype() == 6
    })
}

#[inline(never)]
fn load(
    flash: &mut FlashStorage<'_>,
    scratch: &mut [u8; config::IMAGE_SIZE],
    output: &mut Config,
) -> Option<(u32, u32)> {
    if !has_settings_partition(flash) {
        return None;
    }
    settings::load_banks(
        |offset, bytes| ReadNorFlash::read(flash, offset, bytes),
        scratch,
        output,
    )
}

#[inline(never)]
fn save(
    flash: &mut FlashStorage<'_>,
    scratch: &mut [u8; config::IMAGE_SIZE],
    c: &Config,
    generation: &mut u32,
    bank: &mut u32,
) -> bool {
    let target = config::BANK_SIZE as u32 - *bank;
    let next = generation.wrapping_add(1);
    if config::encode(c, next, scratch).is_err() {
        return false;
    }
    let offset = config::FLASH_OFFSET + target;
    if flash
        .erase(offset, offset + config::IMAGE_SIZE as u32)
        .is_err()
        || flash.write(offset, scratch).is_err()
        || ReadNorFlash::read(flash, offset, scratch).is_err()
    {
        return false;
    }
    if !config::decode(scratch).is_ok_and(|(saved, g)| saved == *c && g == next) {
        return false;
    }
    *bank = target;
    *generation = next;
    true
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::_80MHz));
    let mut rtc = Rtc::new(p.LPWR);
    let wake = wakeup_cause();
    let mut wake_pin = p.GPIO0;
    wake_pin.rtcio_pad_hold(false);
    let mut reset_pin = p.GPIO1;
    reset_pin.rtcio_pad_hold(false);
    esp_alloc::heap_allocator!(size: 64 * 1024);
    let timers = TimerGroup::new(p.TIMG0);
    esp_rtos::start(
        timers.timer0,
        SoftwareInterruptControl::new(p.SW_INTERRUPT).software_interrupt0,
    );
    info!(
        "Boot: starting timetable; reset {:?}",
        esp_hal::system::reset_reason()
    );
    let mut backlight = Output::new(p.GPIO3, Level::Low, OutputConfig::default());
    let mut cs = Output::new(p.GPIO10, Level::Low, OutputConfig::default());
    let dc = Output::new(p.GPIO2, Level::Low, OutputConfig::default());
    let spi = Spi::new(
        p.SPI2,
        SpiConfig::default().with_frequency(Rate::from_mhz(40)),
    )
    .unwrap()
    .with_sck(p.GPIO6)
    .with_mosi(p.GPIO7);
    let mut panel = panel::Panel::new(spi, dc);
    let mut delay = Delay::new();
    panel.init(&mut delay).unwrap();
    let mut touch = I2c::new(
        p.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap()
    .with_sda(p.GPIO4)
    .with_scl(p.GPIO5);
    let mut reset = Output::new(reset_pin.reborrow(), Level::Low, OutputConfig::default());
    delay.delay_millis(20);
    reset.set_high();
    delay.delay_millis(400);
    let _ = touch.write(0x15, &[0xfe, 1]);
    let scratch = SETTINGS.init([0; config::IMAGE_SIZE]);
    if !matches!(wake, SleepSource::Timer) {
        let pixels = scratch.first_chunk_mut::<{ panel::TILE_BYTES }>().unwrap();
        for y in (0..240).step_by(panel::TILE_ROWS) {
            render::starting(&mut panel::Tile { y, pixels }).unwrap();
            panel.write_tile(y, pixels).unwrap();
        }
        backlight.set_high();
    }
    info!("Boot: panel ready; loading settings");
    let mut flash = FlashStorage::new(p.FLASH);
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    // Only this main task accesses persistent bytes; restore validates them
    // before publishing an app. Settings load writes directly into static State.
    let retained = unsafe { &mut *core::ptr::addr_of_mut!(RETAINED) };
    let Some(Boot {
        shared,
        mut generation,
        mut bank,
        restored,
        connect,
    }) = initialize_state(
        &mut flash,
        scratch,
        retained,
        rtc::stamp().0,
        matches!(wake, SleepSource::Gpio | SleepSource::Timer),
    )
    else {
        let pixels = scratch.first_chunk_mut::<{ panel::TILE_BYTES }>().unwrap();
        for y in (0..240).step_by(panel::TILE_ROWS) {
            render::setup(&mut panel::Tile { y, pixels }).unwrap();
            panel.write_tile(y, pixels).unwrap();
        }
        backlight.set_high();
        loop {
            Timer::after_secs(60).await;
        }
    };
    if let Some(utc) = restored.as_ref().and_then(|r| r.utc_micros) {
        transport::restore_clock(utc);
    }
    let mut next_refresh_rtc = rtc
        .time_since_power_up()
        .as_micros()
        .saturating_add(restored.as_ref().map_or(0, |r| r.refresh_in_us));
    let wake_ui = battery::wake_ui(
        restored.as_ref(),
        matches!(wake, SleepSource::Timer),
        matches!(wake, SleepSource::Gpio),
    );
    let mut visible = wake_ui.visible;
    let mut suppress_touch = wake_ui.suppress_touch;
    let mut radio = Some(p.WIFI);
    if connect {
        info!("Boot: settings ready; starting network");
        start_network(spawner, radio.take().unwrap(), shared);
    }
    info!("Boot: application ready");
    log_heap();
    let mut down_at = None;
    let mut start = (0u16, 0u16);
    let mut end = start;
    let mut animation = 0_u8;
    let mut redraw = Instant::now();
    let mut last_activity = redraw;
    loop {
        let mut data = [0; 5];
        let now = Instant::now();
        if touch.write_read(0x15, &[0x02], &mut data).is_ok() {
            let down = data[0] & 0x0f != 0;
            if down {
                last_activity = now;
                if !visible {
                    visible = true;
                    suppress_touch = true;
                    redraw = now;
                }
                end = (
                    ((data[1] as u16 & 15) << 8) | data[2] as u16,
                    ((data[3] as u16 & 15) << 8) | data[4] as u16,
                );
                if down_at.is_none() {
                    start = end;
                    down_at = Some(now);
                }
            } else if suppress_touch {
                down_at = None;
                suppress_touch = false;
            } else if let Some(pressed) = down_at.take() {
                last_activity = now;
                let mut s = shared.lock().await;
                s.app.sleep_failed = false;
                let mut request = false;
                let State {
                    config: c,
                    app,
                    revision,
                    ..
                } = &mut *s;
                let dx = end.0 as i16 - start.0 as i16;
                let dy = end.1 as i16 - start.1 as i16;
                if dx.unsigned_abs() > 35 || dy.unsigned_abs() > 35 {
                    ui::swipe(app, c, &catalog, dx, dy);
                } else if end.0 < 240 && end.1 < 240 {
                    match ui::tap(
                        app,
                        c,
                        &catalog,
                        end.0,
                        end.1,
                        now.duration_since(pressed) >= Duration::from_millis(700),
                    ) {
                        ui::Action::Save { refresh } => {
                            app.save_failed =
                                !save(&mut flash, scratch, c, &mut generation, &mut bank);
                            if app.save_failed {
                                warn!("Settings could not be saved");
                            }
                            if refresh {
                                *revision = revision.wrapping_add(1);
                                request = true;
                            }
                        }
                        ui::Action::Refresh => request = true,
                        ui::Action::None => {}
                    }
                }
                if request {
                    s.network_busy = true;
                    if let Some(wifi) = radio.take() {
                        start_network(spawner, wifi, shared);
                    } else {
                        REFRESH.signal(());
                    }
                }
                redraw = now;
            }
        }
        if visible && now >= redraw {
            // This task saves settings and draws sequentially in the same RAM.
            let pixels = scratch.first_chunk_mut::<{ panel::TILE_BYTES }>().unwrap();
            let mut s = shared.lock().await;
            if let Some(utc) = transport::now() {
                let State { config: c, app, .. } = &mut *s;
                app.tick(c, utc);
            }
            for y in (0..240).step_by(panel::TILE_ROWS) {
                render::draw_animated(
                    &mut panel::Tile { y, pixels },
                    &s.app,
                    &s.config,
                    &catalog,
                    animation,
                )
                .unwrap();
                panel.write_tile(y, pixels).unwrap();
                embassy_futures::yield_now().await;
            }
            backlight.set_high();
            animation = animation.wrapping_add(1);
            redraw = Instant::now()
                + if matches!(
                    s.app.status,
                    Status::Connecting | Status::Syncing | Status::Loading
                ) {
                    Duration::from_millis(500)
                } else {
                    Duration::from_secs(1)
                };
        }
        {
            let mut s = shared.lock().await;
            if s.config.battery
                && battery::refresh_due(
                    radio.is_some(),
                    s.powering_down,
                    rtc.time_since_power_up().as_micros(),
                    next_refresh_rtc,
                )
            {
                s.network_busy = true;
                start_network(spawner, radio.take().unwrap(), shared);
            }
            if let Some(seconds) = s.completed_delay.take() {
                next_refresh_rtc = rtc
                    .time_since_power_up()
                    .as_micros()
                    .saturating_add(u64::from(seconds) * 1_000_000);
            }
            if s.config.battery
                && battery::can_sleep(
                    &s.app,
                    false,
                    now.duration_since(last_activity).as_secs(),
                    down_at.is_some(),
                )
            {
                visible = false;
                backlight.set_low();
            }
            if s.config.battery
                && battery::can_sleep(
                    &s.app,
                    s.network_busy,
                    now.duration_since(last_activity).as_secs(),
                    down_at.is_some(),
                )
            {
                // Keep touch sensing alive: standby A5=3 would require reset to
                // wake and cannot serve as a tap wake source.
                let mut check = [0];
                let touch_ready = touch.write(0x15, &[0xfa, 0x40]).is_ok()
                    && touch.write(0x15, &[0xfe, 0]).is_ok()
                    && touch.write_read(0x15, &[0xfa], &mut check).is_ok()
                    && check[0] == 0x40
                    && touch.write_read(0x15, &[0xfe], &mut check).is_ok()
                    && check[0] == 0;
                let released = {
                    let input = Input::new(
                        wake_pin.reborrow(),
                        InputConfig::default().with_pull(Pull::Up),
                    );
                    input.is_high()
                };
                config::encode(&s.config, generation, scratch).unwrap();
                let tag = u32::from_le_bytes(scratch[16..20].try_into().unwrap());
                let (ticks, period) = rtc::stamp();
                let remaining =
                    next_refresh_rtc.saturating_sub(rtc.time_since_power_up().as_micros());
                let saved = battery::save(
                    retained,
                    &s.app,
                    tag,
                    ticks,
                    period,
                    transport::now_micros(),
                    remaining,
                )
                .is_ok();
                if touch_ready && released && saved {
                    s.powering_down = true;
                    drop(s);
                    if radio.is_none() {
                        POWER_DOWN.signal(());
                        if with_timeout(Duration::from_secs(5), RADIO_OFF.wait())
                            .await
                            .is_err()
                        {
                            warn!(
                                "Radio shutdown pending; CPU remains awake until acknowledgement"
                            );
                            let mut s = shared.lock().await;
                            s.app.sleep_failed = true;
                            let pixels =
                                scratch.first_chunk_mut::<{ panel::TILE_BYTES }>().unwrap();
                            for y in (0..240).step_by(panel::TILE_ROWS) {
                                render::draw(
                                    &mut panel::Tile { y, pixels },
                                    &s.app,
                                    &s.config,
                                    &catalog,
                                )
                                .unwrap();
                                panel.write_tile(y, pixels).unwrap();
                            }
                            backlight.set_high();
                            drop(s);
                            // Controller ownership has been consumed. Resume only
                            // through a fresh boot, never an orphaned REFRESH.
                            RADIO_OFF.wait().await;
                        }
                    }
                    let mut fingers = [0];
                    let touched = touch.write_read(0x15, &[0x02], &mut fingers).is_ok()
                        && fingers[0] & 15 != 0;
                    if touched {
                        battery::show_on_wake(retained);
                    }
                    backlight.set_low();
                    if panel.sleep(&mut delay).is_err() {
                        warn!("Panel sleep command failed");
                    }
                    cs.set_high();
                    reset_pin.rtcio_pad_hold(true);
                    let mut pins: [(&mut dyn RtcPinWithResistors, WakeupLevel); 1] =
                        [(&mut wake_pin, WakeupLevel::Low)];
                    let touch_wake = RtcioWakeupSource::new(&mut pins);
                    let timer =
                        TimerWakeupSource::new(core::time::Duration::from_micros(if touched {
                            1_000
                        } else {
                            remaining.max(60_000_000)
                        }));
                    info!(
                        "Battery sleep; next refresh in {} seconds",
                        remaining / 1_000_000
                    );
                    rtc.sleep_deep(&[&touch_wake, &timer]);
                } else {
                    warn!("Sleep preparation failed; staying awake");
                    s.app.sleep_failed = true;
                    visible = true;
                    redraw = now;
                    last_activity = now;
                    // Restore the active polling mode if preparation failed.
                    let _ = touch.write(0x15, &[0xfe, 1]);
                }
            }
        }
        Timer::after_millis(20).await;
    }
}

// Finish large State/App construction before loading or parsing any settings.
#[inline(never)]
fn create_state() -> &'static Shared {
    STATE.init_with(|| {
        Mutex::new(State {
            config: Config::default(),
            app: App::default(),
            revision: 0,
            network_busy: false,
            completed_delay: None,
            powering_down: false,
        })
    })
}

struct Boot {
    shared: &'static Shared,
    generation: u32,
    bank: u32,
    restored: Option<battery::Wake>,
    connect: bool,
}
#[inline(never)]
fn initialize_state(
    flash: &mut FlashStorage<'_>,
    scratch: &mut [u8; config::IMAGE_SIZE],
    retained: &mut [u8; battery::RETAINED_BYTES],
    rtc_ticks: u64,
    can_restore: bool,
) -> Option<Boot> {
    let shared = create_state();
    let mut s = shared.try_lock().unwrap();
    let (generation, bank) = load(flash, scratch, &mut s.config)?;
    config::encode(&s.config, generation, scratch).ok()?;
    let tag = u32::from_le_bytes(scratch[16..20].try_into().unwrap());
    let restored = if s.config.battery && can_restore {
        let State { config, app, .. } = &mut *s;
        battery::restore(retained, config, tag, rtc_ticks, app).ok()
    } else {
        None
    };
    let connect = !s.config.battery || restored.as_ref().is_none_or(|r| r.refresh_in_us == 0);
    s.network_busy = connect;
    drop(s);
    Some(Boot {
        shared,
        generation,
        bank,
        restored,
        connect,
    })
}

#[inline(never)]
fn start_network(
    spawner: Spawner,
    wifi: esp_hal::peripherals::WIFI<'static>,
    shared: &'static Shared,
) {
    spawner.spawn(
        network_service(
            wifi,
            shared,
            NET.init(StackResources::new()),
            QUERY_CONFIG.take(),
            FEED.init_with(|| Builder::new("20261017".parse().unwrap())),
            WORKSPACE.take(),
        )
        .unwrap(),
    );
    info!("Wi-Fi: update task started; CPU 80 MHz; radio sleeps between updates");
}

#[embassy_executor::task]
async fn network_service(
    mut wifi: esp_hal::peripherals::WIFI<'static>,
    shared: &'static Shared,
    resources: &'static mut StackResources<4>,
    query_config: &'static mut Config,
    builder: &'static mut Builder,
    workspace: &'static mut gtfs::zip::Workspace,
) {
    loop {
        {
            let mut state = match power::session(shared.lock(), POWER_DOWN.wait()).await {
                Either::Second(state) => state,
                Either::First(()) => break,
            };
            if state.powering_down {
                break;
            }
            state.network_busy = true;
        }
        // Cancelling the entire session drops the runner, sockets and controller
        // before RADIO_OFF can acknowledge shutdown. The owned WIFI token is
        // safely reborrowed for the next session; no stolen peripheral handles.
        let delay = match power::session(
            network_session(
                wifi.reborrow(),
                resources,
                shared,
                query_config,
                builder,
                workspace,
            ),
            POWER_DOWN.wait(),
        )
        .await
        {
            Either::Second(delay) => delay,
            Either::First(()) => break,
        };
        info!(
            "Wi-Fi: radio off; next automatic update in {} seconds",
            delay
        );
        log_heap();
        if delay == 0 {
            continue;
        }
        {
            let mut state = shared.lock().await;
            state.network_busy = false;
            state.completed_delay = Some(if delay == battery::REFRESH_SECONDS as u32 {
                delay
            } else {
                battery::RETRY_SECONDS as u32
            });
        }
        if power::idle(
            Timer::after_secs(u64::from(delay)),
            REFRESH.wait(),
            POWER_DOWN.wait(),
        )
        .await
            == power::Wake::Shutdown
        {
            break;
        }
    }
    RADIO_OFF.signal(());
}

#[inline(never)]
fn open_network<'d>(
    wifi: esp_hal::peripherals::WIFI<'d>,
    resources: &'d mut StackResources<4>,
) -> Result<(WifiController<'d>, Stack<'d>, Runner<'d, Interface<'d>>), WifiError> {
    info!("Wi-Fi: initializing radio");
    let config = esp_radio::wifi::ControllerConfig::default()
        .with_static_rx_buf_num(2)
        .with_dynamic_rx_buf_num(2)
        .with_dynamic_tx_buf_num(2)
        .with_rx_queue_size(2)
        .with_tx_queue_size(2)
        .with_rx_ba_win(0)
        .with_ampdu_rx_enable(false)
        .with_ampdu_tx_enable(false);
    let (mut controller, interfaces) = esp_radio::wifi::new(wifi, config)?;
    match controller.set_power_saving(PowerSaveMode::None) {
        Ok(()) => info!("Wi-Fi: radio initialized; transfer mode, modem power save off"),
        Err(error) => log_wifi_error("transfer power mode unavailable", error),
    }
    let mut seed = [0; 8];
    Rng::new().read(&mut seed);
    let (stack, runner) = embassy_net::new(
        interfaces.station,
        embassy_net::Config::dhcpv4(Default::default()),
        resources,
        u64::from_le_bytes(seed),
    );
    Ok((controller, stack, runner))
}

async fn network_session(
    wifi: esp_hal::peripherals::WIFI<'_>,
    resources: &mut StackResources<4>,
    shared: &'static Shared,
    query_config: &mut Config,
    builder: &mut Builder,
    workspace: &mut gtfs::zip::Workspace,
) -> u32 {
    let (controller, stack, mut runner) = match open_network(wifi, resources) {
        Ok(network) => network,
        Err(error) => {
            log_wifi_error("initialization failed", error);
            shared.lock().await.app.status = Status::Offline;
            return 30;
        }
    };
    // The driver stays alive only while a query is in progress. An intentional
    // idle shutdown does not turn a successfully cached board into Offline.
    match select(
        runner.run(),
        select(
            wifi_connection(controller, shared),
            update_once(stack, shared, query_config, builder, workspace),
        ),
    )
    .await
    {
        Either::First(_) => unreachable!(),
        Either::Second(Either::First(_)) => {
            shared.lock().await.app.status = Status::Offline;
            30
        }
        Either::Second(Either::Second(delay)) => delay,
    }
}

fn log_wifi_error(stage: &str, error: WifiError) {
    match error {
        WifiError::Disconnected(info) => warn!(
            "Wi-Fi {}: reason {:?}, signal {} dBm",
            stage, info.reason, info.rssi
        ),
        other => warn!("Wi-Fi {}: {:?}", stage, other),
    }
}

async fn wifi_connection(
    mut controller: WifiController<'_>,
    shared: &'static Shared,
) -> Result<(), WifiError> {
    let station = {
        let state = shared.lock().await;
        let c = &state.config;
        StationConfig::default()
            .with_ssid(c.wifi_ssid.as_str())
            .with_auth_method(if c.wifi_password.is_empty() {
                AuthenticationMethod::None
            } else {
                AuthenticationMethod::Wpa2Personal
            })
            .with_password(c.wifi_password.as_str().into())
    };
    info!("Wi-Fi: applying station configuration");
    controller
        .set_config(&WifiConfig::Station(station))
        .inspect_err(|e| log_wifi_error("configuration failed", *e))?;
    info!("Wi-Fi: station configured");
    loop {
        info!("Wi-Fi: requesting association");
        match controller.connect_async().await {
            Ok(_) => {
                info!("Wi-Fi associated");
                match controller.wait_for_disconnect_async().await {
                    Ok(info) => warn!(
                        "Wi-Fi disconnected: reason {:?}, signal {} dBm",
                        info.reason, info.rssi
                    ),
                    Err(error) => log_wifi_error("disconnect", error),
                }
            }
            Err(error) => log_wifi_error("association failed", error),
        }
        shared.lock().await.app.status = Status::Offline;
        Timer::after_secs(3).await;
    }
}

// Keep the settings clone's temporary frame off the later TLS call path.
#[inline(never)]
fn copy_query(state: &State, out: &mut Config) -> u32 {
    out.clone_from(&state.config);
    state.revision
}

async fn update_once(
    stack: Stack<'_>,
    shared: &'static Shared,
    query_config: &mut Config,
    builder: &mut Builder,
    workspace: &mut gtfs::zip::Workspace,
) -> u32 {
    {
        let mut s = shared.lock().await;
        if s.powering_down {
            return 0;
        }
        s.network_busy = true;
    }
    info!("Network: waiting up to 30 seconds for IPv4/DHCP");
    if with_timeout(Duration::from_secs(30), stack.wait_config_up())
        .await
        .is_err()
    {
        warn!(
            "Network: IPv4/DHCP timeout; Wi-Fi link up: {}",
            stack.is_link_up()
        );
        log_heap();
        let mut s = shared.lock().await;
        s.app.status = Status::Offline;
        drop(s);
        return 30;
    }
    info!("Network: IPv4 configuration ready; synchronizing clock");
    shared.lock().await.app.status = Status::Syncing;
    if transport::synchronize(stack).await.is_err() {
        warn!("Clock synchronization failed; retrying in 30 seconds");
        let mut s = shared.lock().await;
        s.app.status = Status::Offline;
        drop(s);
        return 30;
    }
    info!("Clock synchronized; starting timetable update");
    let revision = {
        let mut s = shared.lock().await;
        s.app.status = Status::Loading;
        copy_query(&s, query_config)
    };
    let result = select(
        timetable_firmware::download::refresh(
            &Http(stack),
            builder,
            workspace,
            query_config,
            transport::now().unwrap(),
        ),
        REFRESH.wait(),
    )
    .await;
    log_heap();
    let result = match result {
        Either::First(r) => r,
        Either::Second(()) => return 0,
    };
    let successful = result.is_ok();
    {
        let mut s = shared.lock().await;
        if s.revision != revision {
            return 0;
        }
        match result {
            Ok(mut freshness) => {
                if freshness.checked_at.is_some() {
                    freshness.checked_at = transport::now();
                }
                s.app.publish(
                    core::mem::take(&mut builder.events),
                    core::mem::take(&mut builder.headsigns),
                    builder.now,
                );
                s.app.freshness = freshness;
            }
            Err(Error::Feed(gtfs::Error::IndexFull | gtfs::Error::IdTooLong)) => {
                s.app.status = Status::Capacity
            }
            Err(error) => {
                warn!("Timetable update failed: {error:?}");
                s.app.status = Status::Failed;
            }
        }
    }
    if successful {
        battery::REFRESH_SECONDS as u32
    } else {
        60
    }
}
