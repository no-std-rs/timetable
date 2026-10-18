#![no_std]
#![no_main]
extern crate alloc;
mod transport;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{Runner, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::Read;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    interrupt::software::SoftwareInterruptControl,
    rng::Rng,
    spi::master::{Config as SpiConfig, Spi},
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{
    AuthenticationMethod, Config as WifiConfig, Interface, WifiController, sta::StationConfig,
};
use esp_storage::FlashStorage;
use log::{info, warn};
use static_cell::StaticCell;
use timetable_display::{
    App, Config, catalog::Catalog, config, feed::Builder, model::Status, render, ui,
};
use timetable_firmware::{panel, sources::Archives};
use transport::{Error, Http, Operation};

struct State {
    config: Config,
    app: App,
    revision: u32,
}
type Shared = Mutex<CriticalSectionRawMutex, State>;
static STATE: StaticCell<Shared> = StaticCell::new();
static NET: StaticCell<StackResources<4>> = StaticCell::new();
static SETTINGS: StaticCell<[u8; config::IMAGE_SIZE]> = StaticCell::new();
const _: () = assert!(panel::TILE_BYTES <= config::IMAGE_SIZE);
static FEED: StaticCell<Builder> = StaticCell::new();
static WORKSPACE: StaticCell<gtfs::zip::Workspace> = StaticCell::new();
static REFRESH: Signal<CriticalSectionRawMutex, ()> = Signal::new();
esp_bootloader_esp_idf::esp_app_desc!();

fn log_heap() {
    let stats = esp_alloc::HEAP.stats();
    info!(
        "Heap usage: {} B current / {} B peak / {} B capacity",
        stats.current_usage, stats.max_usage, stats.size
    );
}

fn load(
    flash: &mut FlashStorage<'_>,
    scratch: &mut [u8; config::IMAGE_SIZE],
) -> Option<(Config, u32, u32)> {
    use esp_bootloader_esp_idf::partitions::{PARTITION_TABLE_MAX_LEN, read_partition_table};
    let mut table = [0; PARTITION_TABLE_MAX_LEN];
    let table = read_partition_table(flash, &mut table).ok()?;
    table.iter().find(|p| {
        p.label().starts_with(b"timetable\0")
            && p.offset() == config::FLASH_OFFSET
            && p.len() == 0x10000
            && p.raw_type() == 1
            && p.raw_subtype() == 6
    })?;
    let mut chosen: Option<(Config, u32, u32)> = None;
    for bank in [0, config::BANK_SIZE as u32] {
        if ReadNorFlash::read(flash, config::FLASH_OFFSET + bank, scratch).is_ok()
            && let Ok((c, g)) = config::decode(scratch)
            && chosen
                .as_ref()
                .is_none_or(|(_, old, _)| config::newer(g, *old))
        {
            chosen = Some((c, g, bank));
        }
    }
    chosen
}

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
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(size: 64 * 1024);
    let timers = TimerGroup::new(p.TIMG0);
    esp_rtos::start(
        timers.timer0,
        SoftwareInterruptControl::new(p.SW_INTERRUPT).software_interrupt0,
    );
    let _backlight = Output::new(p.GPIO3, Level::High, OutputConfig::default());
    let _cs = Output::new(p.GPIO10, Level::Low, OutputConfig::default());
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
    let mut reset = Output::new(p.GPIO1, Level::Low, OutputConfig::default());
    delay.delay_millis(20);
    reset.set_high();
    delay.delay_millis(400);
    let _ = touch.write(0x15, &[0xfe, 1]);
    let scratch = SETTINGS.init([0; config::IMAGE_SIZE]);
    let mut flash = FlashStorage::new(p.FLASH);
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let Some((settings, mut generation, mut bank)) = load(&mut flash, scratch) else {
        let pixels = scratch.first_chunk_mut::<{ panel::TILE_BYTES }>().unwrap();
        for y in (0..240).step_by(panel::TILE_ROWS) {
            render::setup(&mut panel::Tile { y, pixels }).unwrap();
            panel.write_tile(y, pixels).unwrap();
        }
        loop {
            Timer::after_secs(60).await;
        }
    };
    let shared = STATE.init(Mutex::new(State {
        config: settings,
        app: App::default(),
        revision: 0,
    }));
    let (controller, interfaces) = esp_radio::wifi::new(p.WIFI, Default::default()).unwrap();
    let mut seed = [0; 8];
    Rng::new().read(&mut seed);
    let (stack, runner) = embassy_net::new(
        interfaces.station,
        embassy_net::Config::dhcpv4(Default::default()),
        NET.init(StackResources::new()),
        u64::from_le_bytes(seed),
    );
    spawner.spawn(wifi_connection(controller, shared).unwrap());
    spawner.spawn(network(runner).unwrap());
    spawner.spawn(
        timetable(
            stack,
            shared,
            FEED.init_with(|| Builder::new("20261017".parse().unwrap())),
            WORKSPACE.init_with(gtfs::zip::Workspace::new),
        )
        .unwrap(),
    );
    log_heap();
    let mut down_at = None;
    let mut start = (0u16, 0u16);
    let mut end = start;
    let mut redraw = Instant::now();
    loop {
        let mut data = [0; 5];
        let now = Instant::now();
        if touch.write_read(0x15, &[0x02], &mut data).is_ok() {
            let down = data[0] & 0x0f != 0;
            if down {
                end = (
                    ((data[1] as u16 & 15) << 8) | data[2] as u16,
                    ((data[3] as u16 & 15) << 8) | data[4] as u16,
                );
                if down_at.is_none() {
                    start = end;
                    down_at = Some(now);
                }
            } else if let Some(pressed) = down_at.take() {
                let mut s = shared.lock().await;
                let State {
                    config: c,
                    app,
                    revision,
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
                                REFRESH.signal(());
                            }
                        }
                        ui::Action::Refresh => REFRESH.signal(()),
                        ui::Action::None => {}
                    }
                }
                redraw = now;
            }
        }
        if now >= redraw {
            // This task saves settings and draws sequentially in the same RAM.
            let pixels = scratch.first_chunk_mut::<{ panel::TILE_BYTES }>().unwrap();
            let mut s = shared.lock().await;
            if let Some(utc) = transport::now() {
                let State { config: c, app, .. } = &mut *s;
                app.tick(c, utc);
            }
            for y in (0..240).step_by(panel::TILE_ROWS) {
                render::draw(&mut panel::Tile { y, pixels }, &s.app, &s.config, &catalog).unwrap();
                panel.write_tile(y, pixels).unwrap();
                embassy_futures::yield_now().await;
            }
            redraw = Instant::now() + Duration::from_secs(1);
        }
        Timer::after_millis(20).await;
    }
}

#[embassy_executor::task]
async fn wifi_connection(mut controller: WifiController<'static>, shared: &'static Shared) -> ! {
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
    controller
        .set_config(&WifiConfig::Station(station))
        .unwrap();
    loop {
        match controller.connect_async().await {
            Ok(_) => {
                info!("Wi-Fi associated");
                let _ = controller.wait_for_disconnect_async().await;
            }
            Err(_) => warn!("Wi-Fi association failed"),
        }
        shared.lock().await.app.status = Status::Offline;
        Timer::after_secs(3).await;
    }
}
#[embassy_executor::task]
async fn network(mut runner: Runner<'static, Interface<'static>>) -> ! {
    runner.run().await
}

impl Operation for Archives {
    async fn read<R: Read>(&mut self, body: R) -> Result<(), Error> {
        let mut listing = gtfs::poznan::Archives::new(body);
        while let Some(name) = listing.next().await? {
            self.consider(name);
        }
        Ok(())
    }
}
enum Phase {
    Calendars(u8),
    Trips,
    Departures,
}
struct Pass<'a> {
    builder: &'a mut Builder,
    workspace: &'a mut gtfs::zip::Workspace,
    config: &'a Config,
    phase: Phase,
}
impl Operation for Pass<'_> {
    async fn read<R: Read>(&mut self, body: R) -> Result<(), Error> {
        match self.phase {
            Phase::Calendars(mask) => {
                self.builder
                    .calendars(body, mask, self.config, self.workspace)
                    .await?
            }
            Phase::Trips => {
                self.builder
                    .trips(body, self.config, self.workspace)
                    .await?
            }
            Phase::Departures => {
                self.builder
                    .departures(body, self.config, self.workspace)
                    .await?
            }
        }
        Ok(())
    }
}
async fn refresh(
    http: &Http,
    builder: &mut Builder,
    workspace: &mut gtfs::zip::Workspace,
    c: &Config,
) -> Result<(), Error> {
    builder.begin(transport::now().ok_or(Error::Clock)?)?;
    if c.groups.iter().all(|g| g.picks.is_empty()) {
        return Ok(());
    }
    let mut listing = Archives::new(builder.dates, builder.day_mask());
    http.get(gtfs::poznan::LIST_URL, &mut listing).await?;
    let sources = listing.sources().ok_or(Error::MissingArchive)?;
    for (name, mask) in sources {
        if mask == 0 {
            continue;
        }
        let name = name.ok_or(Error::MissingArchive)?;
        let mut url = [0; 192];
        let url = name.url(&mut url)?;
        for phase in [Phase::Calendars(mask), Phase::Trips, Phase::Departures] {
            http.get(
                url,
                &mut Pass {
                    builder,
                    workspace,
                    config: c,
                    phase,
                },
            )
            .await?;
        }
    }
    Ok(())
}
#[embassy_executor::task]
async fn timetable(
    stack: Stack<'static>,
    shared: &'static Shared,
    builder: &'static mut Builder,
    workspace: &'static mut gtfs::zip::Workspace,
) -> ! {
    loop {
        stack.wait_config_up().await;
        shared.lock().await.app.status = Status::Syncing;
        if transport::synchronize(stack).await.is_err() {
            shared.lock().await.app.status = Status::Offline;
            let _ = select(Timer::after_secs(30), REFRESH.wait()).await;
            continue;
        }
        let (c, revision) = {
            let mut s = shared.lock().await;
            s.app.status = Status::Loading;
            (s.config.clone(), s.revision)
        };
        let result = select(
            refresh(&Http(stack), builder, workspace, &c),
            REFRESH.wait(),
        )
        .await;
        log_heap();
        let result = match result {
            Either::First(r) => r,
            Either::Second(()) => continue,
        };
        let successful = result.is_ok();
        {
            let mut s = shared.lock().await;
            if s.revision != revision {
                continue;
            }
            match result {
                Ok(()) => {
                    s.app.publish(
                        core::mem::take(&mut builder.events),
                        core::mem::take(&mut builder.headsigns),
                        builder.now,
                    );
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
        let _ = select(
            Timer::after_secs(if successful { 3 * 3600 } else { 60 }),
            REFRESH.wait(),
        )
        .await;
    }
}
