#![no_std]
#![no_main]
extern crate alloc;
mod catalog_slot;
#[cfg(feature = "http-probe")]
mod http_probe;
mod http_request;
mod http_session;
mod rtc;
mod scheduler;
mod tls_cipher;
mod tls_memory;
mod transport;
mod wake_gate;

use core::cell::{Cell, RefCell};
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{Runner, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_backtrace as _;
#[cfg(feature = "board-waveshare-c6")]
use esp_hal::ledc::{
    LSGlobalClkSource, Ledc, LowSpeed,
    channel::{self, ChannelIFace},
    timer::{self, TimerIFace},
};
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    interrupt::software::SoftwareInterruptControl,
    rng::Rng,
    rtc_cntl::{Rtc, wakeup_cause},
    spi::master::{Config as SpiConfig, Spi},
    system::SleepSource,
    time::Rate,
    timer::timg::TimerGroup,
};
#[cfg(not(feature = "board-waveshare-c6"))]
use esp_hal::{
    gpio::{Input, InputConfig, Pull, RtcPin, RtcPinWithResistors},
    rtc_cntl::sleep::{RtcioWakeupSource, TimerWakeupSource, WakeupLevel},
};
use esp_radio::wifi::{
    AuthenticationMethod, Config as WifiConfig, Interface, PowerSaveMode, WifiController,
    WifiError, sta::StationConfig,
};
use esp_storage::FlashStorage;
use log::{info, warn};
use static_cell::{ConstStaticCell, StaticCell};
use timetable_display::{
    App, Config, config,
    feed::Builder,
    model::{CacheUpdate, Status},
    render, ui,
};
use timetable_firmware::{battery, cache, panel, power, settings};
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
type FlashMutex =
    embassy_sync::blocking_mutex::Mutex<CriticalSectionRawMutex, RefCell<FlashStorage<'static>>>;
static FLASH: StaticCell<FlashMutex> = StaticCell::new();
#[derive(Clone, Copy)]
struct CachePartition {
    flash: &'static FlashMutex,
    enabled: bool,
}

struct CacheFlash {
    flash: &'static FlashMutex,
    shared: &'static Shared,
    revision: u32,
    approval: Cell<Option<i64>>,
}
impl CacheFlash {
    fn bounds(offset: u32, length: usize) -> Result<(), gtfs::Error> {
        if offset < cache::OFFSET
            || offset
                .checked_add(length as u32)
                .is_none_or(|end| end > cache::OFFSET + cache::SIZE)
        {
            return Err(gtfs::Error::InvalidRange);
        }
        Ok(())
    }
}
impl cache::Storage for CacheFlash {
    fn metadata(&self) -> Result<impl AsMut<[cache::Metadata]>, gtfs::Error> {
        // Bounded metadata inside the existing 64 KiB heap, after verifier
        // allocations have been freed. Allocation failure uses normal streaming.
        let mut metadata = alloc::vec::Vec::new();
        metadata
            .try_reserve_exact(1)
            .map_err(|_| gtfs::Error::BufferTooSmall)?;
        metadata.push(cache::Metadata::default());
        Ok(metadata)
    }
    fn identity(&self) -> &[u8] {
        catalog_slot::profile().identity()
    }
    fn scratch(&self) -> Result<impl AsMut<[u8]>, gtfs::Error> {
        let mut bytes = alloc::vec::Vec::new();
        bytes
            .try_reserve_exact(cache::FILTER_BYTES)
            .map_err(|_| gtfs::Error::BufferTooSmall)?;
        bytes.resize(cache::FILTER_BYTES, 0);
        Ok(bytes)
    }
    fn read(&self, offset: u32, out: &mut [u8]) -> Result<(), gtfs::Error> {
        Self::bounds(offset, out.len())?;
        self.flash
            .lock(|flash| ReadNorFlash::read(&mut *flash.borrow_mut(), offset, out))
            .map_err(|_| gtfs::Error::Io(embedded_io_async::ErrorKind::Other))
    }
    fn write(&self, offset: u32, bytes: &[u8]) -> Result<(), gtfs::Error> {
        Self::bounds(offset, bytes.len())?;
        self.flash
            .lock(|flash| flash.borrow_mut().write(offset, bytes))
            .map_err(|_| gtfs::Error::Io(embedded_io_async::ErrorKind::Other))
    }
    fn erase(&self, start: u32, end: u32) -> Result<(), gtfs::Error> {
        Self::bounds(
            start,
            end.checked_sub(start).ok_or(gtfs::Error::InvalidRange)? as usize,
        )?;
        self.flash
            .lock(|flash| flash.borrow_mut().erase(start, end))
            .map_err(|_| gtfs::Error::Io(embedded_io_async::ErrorKind::Other))
    }
    async fn current(&self) -> bool {
        let s = self.shared.lock().await;
        s.revision == self.revision && !s.powering_down
    }
    fn checked_at(&self) -> Option<i64> {
        transport::now()
    }
    fn approve_update(&self, previous: i64) -> bool {
        self.approval.take() == Some(previous)
    }
}

impl CacheFlash {
    // The HTTP session has returned before this wait: do not keep TLS buffers
    // or an idle persistent connection alive while the user makes a choice.
    async fn confirm_update(&self) -> bool {
        {
            let mut state = self.shared.lock().await;
            if state.revision != self.revision || state.powering_down {
                return false;
            }
            state.app.cache_update = CacheUpdate::Pending;
        }
        warn!("Cache: flash was updated less than 24 hours ago; asking before another write");
        let accepted = with_timeout(Duration::from_secs(60), async {
            loop {
                {
                    let state = self.shared.lock().await;
                    if state.revision != self.revision || state.powering_down {
                        return false;
                    }
                    match state.app.cache_update {
                        CacheUpdate::Accepted => return true,
                        CacheUpdate::Pending => {}
                        _ => return false,
                    }
                }
                Timer::after_millis(50).await;
            }
        })
        .await
        .unwrap_or(false);
        self.shared.lock().await.app.cache_update = CacheUpdate::None;
        accepted
    }
}

#[inline(never)]
fn has_cache_partition(flash: &mut FlashStorage<'_>) -> bool {
    use esp_bootloader_esp_idf::partitions::{PARTITION_TABLE_MAX_LEN, read_partition_table};
    let mut bytes = [0; PARTITION_TABLE_MAX_LEN];
    let Ok(table) = read_partition_table(flash, &mut bytes) else {
        return false;
    };
    table.iter().any(|p| {
        p.label().starts_with(b"ttcache\0")
            && p.raw_type() == 0x40
            && p.raw_subtype() == 0
            && p.offset() == cache::OFFSET
            && p.len() == cache::SIZE
    }) && table.iter().all(|p| {
        p.label().starts_with(b"ttcache\0")
            || p.offset().checked_add(p.len()).is_some_and(|end| {
                end <= cache::OFFSET || p.offset() >= cache::OFFSET + cache::SIZE
            })
    })
}

static NET: StaticCell<StackResources<4>> = StaticCell::new();
static SETTINGS: StaticCell<[u8; config::IMAGE_SIZE]> = StaticCell::new();
const _: () = assert!(panel::TILE_BYTES <= config::IMAGE_SIZE);
static FEED: ConstStaticCell<Builder> =
    ConstStaticCell::new(Builder::unstarted(match gtfs::Date::new(2000, 1, 1) {
        Ok(date) => date,
        Err(_) => panic!("invalid initial date"),
    }));
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

// Publishing moves the complete departure/headsign caches. Keep those stack
// temporaries out of the async radio caller while it later verifies TLS.
#[inline(never)]
fn publish_departures(app: &mut App, builder: &mut Builder) {
    if let Some(cache) = builder.journeys.as_deref_mut() {
        app.journeys = core::mem::take(cache);
    }
    app.publish(
        core::mem::take(&mut builder.events),
        core::mem::take(&mut builder.headsigns),
        builder.now,
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
    #[allow(unused_mut)]
    let mut rtc = Rtc::new(p.LPWR);
    let wake = wakeup_cause();
    #[cfg(not(feature = "board-waveshare-c6"))]
    let mut wake_pin = p.GPIO0;
    #[cfg(not(feature = "board-waveshare-c6"))]
    wake_pin.rtcio_pad_hold(false);
    #[cfg(not(feature = "board-waveshare-c6"))]
    let mut reset_pin = p.GPIO1;
    #[cfg(not(feature = "board-waveshare-c6"))]
    reset_pin.rtcio_pad_hold(false);
    esp_alloc::heap_allocator!(size: 64 * 1024);
    let timers = TimerGroup::new(p.TIMG0);
    esp_rtos::start(
        timers.timer0,
        SoftwareInterruptControl::new(p.SW_INTERRUPT).software_interrupt0,
    );
    spawner.spawn(wake_relay(&wake_gate::NETWORK).unwrap());
    spawner.spawn(wake_relay(&wake_gate::WORK).unwrap());
    info!(
        "Boot: starting timetable; reset {:?}",
        esp_hal::system::reset_reason()
    );
    info!("CPU: {} MHz", esp_hal::clock::cpu_clock().as_mhz());
    let mut delay = Delay::new();
    #[cfg(not(feature = "board-waveshare-c6"))]
    let (mut panel, mut touch, mut backlight, mut cs, _reset) = {
        let backlight = Output::new(p.GPIO3, Level::Low, OutputConfig::default());
        let cs = Output::new(p.GPIO10, Level::Low, OutputConfig::default());
        let dc = Output::new(p.GPIO2, Level::Low, OutputConfig::default());
        let spi = Spi::new(
            p.SPI2,
            SpiConfig::default().with_frequency(Rate::from_mhz(40)),
        )
        .unwrap()
        .with_sck(p.GPIO6)
        .with_mosi(p.GPIO7);
        let panel = panel::Panel::new(spi, dc);

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
        (panel, touch, backlight, cs, reset)
    };
    #[cfg(not(feature = "board-waveshare-c6"))]
    let mut set_backlight = |on| {
        if on {
            backlight.set_high();
        } else {
            backlight.set_low();
        }
    };
    #[cfg(feature = "board-waveshare-c6")]
    let (mut panel, mut touch, _cs, _lcd_reset, _touch_reset, _sd_cs) = {
        let cs = Output::new(p.GPIO14, Level::Low, OutputConfig::default());
        let dc = Output::new(p.GPIO15, Level::Low, OutputConfig::default());
        let sd_cs = Output::new(p.GPIO4, Level::High, OutputConfig::default());
        let mut lcd_reset = Output::new(p.GPIO22, Level::Low, OutputConfig::default());
        delay.delay_millis(10);
        lcd_reset.set_high();
        delay.delay_millis(10);
        let spi = Spi::new(
            p.SPI2,
            SpiConfig::default().with_frequency(Rate::from_mhz(40)),
        )
        .unwrap()
        .with_sck(p.GPIO1)
        .with_mosi(p.GPIO2);
        let panel = panel::Panel::new(spi, dc);
        let touch = I2c::new(
            p.I2C0,
            I2cConfig::default().with_frequency(Rate::from_khz(400)),
        )
        .unwrap()
        .with_sda(p.GPIO18)
        .with_scl(p.GPIO19);
        let mut reset = Output::new(p.GPIO20, Level::Low, OutputConfig::default());
        delay.delay_millis(10);
        reset.set_high();
        delay.delay_millis(10);
        (panel, touch, cs, lcd_reset, reset, sd_cs)
    };
    #[cfg(feature = "board-waveshare-c6")]
    let mut ledc = Ledc::new(p.LEDC);
    #[cfg(feature = "board-waveshare-c6")]
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);
    #[cfg(feature = "board-waveshare-c6")]
    let mut backlight_timer = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    #[cfg(feature = "board-waveshare-c6")]
    backlight_timer
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty10Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(20),
        })
        .unwrap();
    #[cfg(feature = "board-waveshare-c6")]
    let mut backlight = ledc.channel(channel::Number::Channel0, p.GPIO23);
    #[cfg(feature = "board-waveshare-c6")]
    backlight
        .configure(channel::config::Config {
            timer: &backlight_timer,
            duty_pct: 0,
            drive_mode: esp_hal::gpio::DriveMode::PushPull,
        })
        .unwrap();
    #[cfg(feature = "board-waveshare-c6")]
    let set_backlight = |on| backlight.set_duty(if on { 40 } else { 0 }).unwrap();
    panel.init(&mut delay).unwrap();
    let scratch = SETTINGS.init_with(|| [0; config::IMAGE_SIZE]);
    if !matches!(wake, SleepSource::Timer) {
        let pixels = scratch.first_chunk_mut::<{ panel::TILE_BYTES }>().unwrap();
        for y in (0..panel::HEIGHT as u16).step_by(panel::TILE_ROWS) {
            draw_starting(&mut panel::Tile { y, pixels }).unwrap();
            panel.write_tile(y, pixels).unwrap();
        }
        set_backlight(true);
    }
    info!("Boot: panel ready; loading settings");
    let mut flash = FlashStorage::new(p.FLASH);
    let catalog = catalog_slot::catalog().expect("valid installed catalogue");
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
        for y in (0..panel::HEIGHT as u16).step_by(panel::TILE_ROWS) {
            draw_setup(&mut panel::Tile { y, pixels }).unwrap();
            panel.write_tile(y, pixels).unwrap();
        }
        set_backlight(true);
        loop {
            Timer::after_secs(60).await;
        }
    };
    let cache_enabled = has_cache_partition(&mut flash);
    info!(
        "Cache: dedicated flash partition {}",
        if cache_enabled {
            "ready"
        } else {
            "absent; streaming only"
        }
    );
    let flash = FLASH.init(embassy_sync::blocking_mutex::Mutex::new(RefCell::new(
        flash,
    )));
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
        radio = start_network(spawner, radio.take().unwrap(), shared, flash, cache_enabled);
        if radio.is_some() {
            startup_failed(&mut shared.try_lock().unwrap());
        }
    }
    info!("Boot: application ready");
    log_heap();
    let mut down_at = None;
    let mut start = (0u16, 0u16);
    let mut end = start;
    let mut animation = 0_u8;
    let mut redraw = Instant::now();
    let mut last_activity = redraw;
    let mut confirming_cache = false;
    loop {
        let now = Instant::now();
        let pending = shared.lock().await.app.cache_update == CacheUpdate::Pending;
        if pending != confirming_cache {
            // A gesture must start and finish on the same screen, including
            // when a prompt times out or is cancelled while a finger is down.
            down_at = None;
            suppress_touch = true;
        }
        if pending && !confirming_cache {
            // Wake a battery-mode screen so the question is visible.
            visible = true;
            redraw = now;
        }
        confirming_cache = pending;
        #[cfg(not(feature = "board-waveshare-c6"))]
        let sample = {
            let mut data = [0; 5];
            touch
                .write_read(0x15, &[0x02], &mut data)
                .ok()
                .and_then(|()| timetable_firmware::touch::cst816(&data))
        };
        #[cfg(feature = "board-waveshare-c6")]
        let sample = {
            let mut data = [0; 14];
            touch
                .write(0x63, &[0x01])
                .and_then(|()| touch.read(0x63, &mut data))
                .ok()
                .and_then(|()| decode_touch(&data))
        };
        if let Some(sample) = sample {
            let down = matches!(sample, timetable_firmware::touch::Sample::Pressed { .. });
            if down {
                last_activity = now;
                if !visible {
                    visible = true;
                    suppress_touch = true;
                    redraw = now;
                }
                if let timetable_firmware::touch::Sample::Pressed { x, y } = sample {
                    end = (x, y);
                }
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
                    swipe_screen(app, c, &catalog, dx, dy);
                } else if end.0 < panel::WIDTH as u16 && end.1 < panel::HEIGHT as u16 {
                    match tap_screen(
                        app,
                        c,
                        &catalog,
                        end.0,
                        end.1,
                        now.duration_since(pressed) >= Duration::from_millis(700),
                    ) {
                        ui::Action::Save { refresh } => {
                            app.save_failed = !flash.lock(|flash| {
                                save(
                                    &mut flash.borrow_mut(),
                                    scratch,
                                    c,
                                    &mut generation,
                                    &mut bank,
                                )
                            });
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
                        radio = start_network(spawner, wifi, shared, flash, cache_enabled);
                        if radio.is_some() {
                            startup_failed(&mut s);
                        }
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
                app.tick_in_zone(c, utc, catalog.profile().zone());
            }
            for y in (0..panel::HEIGHT as u16).step_by(panel::TILE_ROWS) {
                let tile_started = Instant::now();
                draw_screen(
                    &mut panel::Tile { y, pixels },
                    &s.app,
                    &s.config,
                    &catalog,
                    animation,
                )
                .unwrap();
                panel.write_tile(y, pixels).unwrap();
                scheduler::tile(tile_started);
                embassy_futures::yield_now().await;
            }
            set_backlight(true);
            animation = animation.wrapping_add(1);
            redraw = Instant::now()
                + if matches!(
                    s.app.status,
                    Status::Restoring | Status::Connecting | Status::Syncing | Status::Loading
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
                radio = start_network(spawner, radio.take().unwrap(), shared, flash, cache_enabled);
                if radio.is_some() {
                    startup_failed(&mut s);
                }
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
                set_backlight(false);
            }
            #[cfg(not(feature = "board-waveshare-c6"))]
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
                let tag =
                    catalog_slot::tag(u32::from_le_bytes(scratch[16..20].try_into().unwrap()));
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
                            for y in (0..panel::HEIGHT as u16).step_by(panel::TILE_ROWS) {
                                render::draw(
                                    &mut panel::Tile { y, pixels },
                                    &s.app,
                                    &s.config,
                                    &catalog,
                                )
                                .unwrap();
                                panel.write_tile(y, pixels).unwrap();
                            }
                            set_backlight(true);
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
                    set_backlight(false);
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

#[embassy_executor::task(pool_size = 2)]
async fn wake_relay(gate: &'static wake_gate::Gate) {
    gate.relay().await;
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
    let tag = catalog_slot::tag(u32::from_le_bytes(scratch[16..20].try_into().unwrap()));
    let restored = if s.config.battery && can_restore {
        let State { config, app, .. } = &mut *s;
        battery::restore(retained, config, tag, rtc_ticks, app).ok()
    } else {
        None
    };
    let connect = !s.config.battery
        || s.config.groups.iter().any(|g| g.journey.is_some())
        || restored.as_ref().is_none_or(|r| r.refresh_in_us == 0);
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

fn heap_slot<T>(value: T) -> Result<alloc::vec::Vec<T>, gtfs::Error> {
    let mut storage = alloc::vec::Vec::new();
    storage
        .try_reserve_exact(1)
        .map_err(|_| gtfs::Error::BufferTooSmall)?;
    storage.push(value);
    Ok(storage)
}
fn startup_failed(state: &mut State) {
    state.network_busy = false;
    state.app.status = Status::Failed;
    state.completed_delay = Some(60);
}
#[inline(never)]
fn start_network(
    spawner: Spawner,
    wifi: esp_hal::peripherals::WIFI<'static>,
    shared: &'static Shared,
    flash: &'static FlashMutex,
    cache_enabled: bool,
) -> Option<esp_hal::peripherals::WIFI<'static>> {
    let (wifi, query_config, journey_cache) = match power::prepare_startup(
        wifi,
        || heap_slot(Config::empty()),
        || heap_slot(timetable_display::journey::Cache::default()),
    ) {
        Ok(slots) => slots,
        Err((wifi, error)) => {
            warn!("Network startup allocation failed: {error:?}; retaining radio for retry");
            return Some(wifi);
        }
    };
    let query_config = query_config.leak().first_mut().unwrap();
    let journey_cache = journey_cache.leak().first_mut().unwrap();
    let builder = FEED.take();
    builder.zone = catalog_slot::profile().zone();
    builder.journeys = Some(journey_cache);
    spawner.spawn(
        network_service(
            wifi,
            shared,
            NET.init(StackResources::new()),
            query_config,
            builder,
            WORKSPACE.take(),
            CachePartition {
                flash,
                enabled: cache_enabled,
            },
        )
        .unwrap(),
    );
    info!("Wi-Fi: update task started; CPU 80 MHz; radio sleeps between updates");
    None
}

#[embassy_executor::task]
async fn network_service(
    mut wifi: esp_hal::peripherals::WIFI<'static>,
    shared: &'static Shared,
    resources: &'static mut StackResources<4>,
    query_config: &'static mut Config,
    builder: &'static mut Builder,
    workspace: &'static mut gtfs::zip::Workspace,
    cache: CachePartition,
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
        let result = power::session(
            network_session(
                wifi.reborrow(),
                resources,
                shared,
                query_config,
                builder,
                workspace,
                &cache,
            ),
            POWER_DOWN.wait(),
        )
        .await;
        // Also clear a prompt when shutdown cancels its future mid-decision.
        shared.lock().await.app.cache_update = CacheUpdate::None;
        let delay = match result {
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

// Replay before initializing Wi-Fi so a cold boot is useful while DHCP/NTP
// are unavailable. Without a clock the renderer labels these as saved times.
async fn restore_saved_board(
    shared: &'static Shared,
    query_config: &mut Config,
    builder: &mut Builder,
    workspace: &mut gtfs::zip::Workspace,
    cache: &CachePartition,
) {
    if !cache.enabled {
        return;
    }
    let revision = {
        let mut state = shared.lock().await;
        if state.app.loaded_at.is_some() || state.powering_down {
            return;
        }
        state.app.status = Status::Restoring;
        copy_query(&state, query_config)
    };
    if query_config.groups.iter().any(|g| g.journey.is_some()) {
        shared.lock().await.app.status = Status::Connecting;
        return;
    }
    let storage = CacheFlash {
        flash: cache.flash,
        shared,
        revision,
        approval: Cell::new(None),
    };
    if let Ok(Some(bank)) = cache::load(&storage).await {
        let reference = transport::now().unwrap_or(bank.checked_at());
        match cache::restore(&storage, bank, builder, workspace, query_config, reference).await {
            Ok(freshness) => {
                let mut state = shared.lock().await;
                if state.revision != storage.revision || state.powering_down {
                    return;
                }
                publish_departures(&mut state.app, builder);
                state.app.freshness = freshness;
                info!(
                    "Cache: saved board ready before Wi-Fi; clock known: {}",
                    transport::now().is_some()
                );
            }
            Err(error) => info!("Cache: saved board unavailable for this selection: {error:?}"),
        }
    }
    let mut state = shared.lock().await;
    if state.revision == storage.revision && !state.powering_down {
        state.app.status = Status::Connecting;
    }
}

async fn network_session(
    wifi: esp_hal::peripherals::WIFI<'_>,
    resources: &mut StackResources<4>,
    shared: &'static Shared,
    query_config: &mut Config,
    builder: &mut Builder,
    workspace: &mut gtfs::zip::Workspace,
    cache: &CachePartition,
) -> u32 {
    restore_saved_board(shared, query_config, builder, workspace, cache).await;
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
    scheduler::reset();
    let mut network = core::pin::pin!(runner.run());
    match select(
        wake_gate::NETWORK.run(scheduler::runner(network.as_mut())),
        wake_gate::WORK.run(select(
            wifi_connection(controller, shared),
            update_once(stack, shared, query_config, builder, workspace, cache),
        )),
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
                match controller.rssi() {
                    Ok(rssi) => info!("Wi-Fi associated; RSSI {} dBm", rssi),
                    Err(_) => info!("Wi-Fi associated; RSSI unavailable"),
                }
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
    cache: &CachePartition,
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
    #[cfg(feature = "http-probe")]
    http_probe::run(stack, workspace).await;
    let revision = {
        let mut s = shared.lock().await;
        s.app.status = Status::Loading;
        copy_query(&s, query_config)
    };
    let storage = CacheFlash {
        flash: cache.flash,
        shared,
        revision,
        approval: Cell::new(None),
    };
    if cache.enabled
        && !query_config.groups.iter().any(|g| g.journey.is_some())
        && let Ok(Some(bank)) = cache::load(&storage).await
    {
        match cache::restore(
            &storage,
            bank,
            builder,
            workspace,
            query_config,
            transport::now().unwrap(),
        )
        .await
        {
            Ok(freshness) => {
                let mut state = shared.lock().await;
                if state.revision != revision || state.powering_down {
                    return 0;
                }
                publish_departures(&mut state.app, builder);
                state.app.freshness = freshness;
                state.app.status = Status::Loading;
                info!("Cache: cold-start departures ready; checking remote identity");
            }
            Err(error) => info!("Cache: no usable selection/date window: {error:?}"),
        }
    }
    let transport = Http(stack);
    let cached = cache::Http {
        inner: &transport,
        flash: &storage,
        enabled: cache.enabled && !query_config.groups.iter().any(|g| g.journey.is_some()),
    };
    let mut confirmed = false;
    let result = loop {
        let result = select(
            timetable_firmware::download::refresh(
                &cached,
                builder,
                workspace,
                query_config,
                transport::now().unwrap(),
            ),
            REFRESH.wait(),
        )
        .await;
        let result = match result {
            Either::First(r) => r,
            Either::Second(()) => return 0,
        };
        let Err(Error::CacheUpdateRequired(previous)) = result else {
            break result;
        };
        if confirmed {
            break Err(Error::CacheUpdateDeclined);
        }
        let decision = select(storage.confirm_update(), REFRESH.wait()).await;
        shared.lock().await.app.cache_update = CacheUpdate::None;
        match decision {
            Either::First(true) => {
                storage.approval.set(Some(previous));
                confirmed = true;
            }
            Either::First(false) => break Err(Error::CacheUpdateDeclined),
            Either::Second(()) => return 0,
        }
    };
    log_heap();
    let successful = result.is_ok();
    let kept_cache = matches!(result, Err(Error::CacheUpdateDeclined));
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
                publish_departures(&mut s.app, builder);
                s.app.freshness = freshness;
            }
            Err(Error::CacheUpdateDeclined) => {
                info!("Cache: keeping saved data; no flash writes; retry at next scheduled check");
                s.app.status = if s.app.loaded_at.is_some() {
                    Status::Ready
                } else {
                    Status::Failed
                };
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
    if successful || kept_cache {
        battery::REFRESH_SECONDS as u32
    } else {
        60
    }
}

#[cfg(all(feature = "board-waveshare-c6", not(feature = "c6-landscape")))]
use render::portrait::{
    draw_animated as draw_screen, setup as draw_setup, starting as draw_starting,
};
#[cfg(not(feature = "board-waveshare-c6"))]
use render::{draw_animated as draw_screen, setup as draw_setup, starting as draw_starting};
#[cfg(not(feature = "board-waveshare-c6"))]
use ui::tap as tap_screen;
#[cfg(all(feature = "board-waveshare-c6", not(feature = "c6-landscape")))]
use ui::tap_portrait as tap_screen;

#[cfg(feature = "c6-landscape")]
use render::landscape::{
    draw_animated as draw_screen, setup as draw_setup, starting as draw_starting,
};
#[cfg(all(feature = "board-waveshare-c6", not(feature = "c6-landscape")))]
use timetable_firmware::touch::axs5106 as decode_touch;
#[cfg(feature = "c6-landscape")]
use timetable_firmware::touch::axs5106_landscape as decode_touch;
#[cfg(feature = "c6-landscape")]
use ui::tap_landscape as tap_screen;

#[cfg(not(feature = "c6-landscape"))]
use ui::swipe as swipe_screen;
#[cfg(feature = "c6-landscape")]
use ui::swipe_landscape as swipe_screen;
