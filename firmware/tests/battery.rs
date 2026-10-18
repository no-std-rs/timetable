use timetable_display::{App, Config, Departure, Screen, config, model::Status};
use timetable_firmware::battery::{self, RETAINED_BYTES};

const BASE: i64 = 2_200_000_000; // Past 2038.
const ANCHOR: u64 = 1_000_000_000_000;
const PERIOD: u32 = 2 << 19; // Two microseconds per synthetic RTC tick.

fn settings() -> Config {
    let mut bytes = *include_bytes!("../../display/tests/settings.bin");
    config::decode(&mut bytes).unwrap().0
}
fn board() -> (Config, App) {
    let mut c = settings();
    let pick = c.groups[0].picks[0].clone();
    c.groups[1].picks.push(pick).unwrap();
    c.groups[0].days = 127;
    c.groups[0].start = 0;
    c.groups[0].end = 0;
    let mut app = App {
        group: 1,
        clock: Some(BASE),
        loaded_at: Some(BASE),
        manual_until: BASE + 7200,
        status: Status::Ready,
        ..App::default()
    };
    app.departures.reset(BASE);
    for i in 0..32 {
        app.headsigns
            .insert(&format!("{i:02}{}", "ą".repeat(15)))
            .unwrap();
    }
    for i in 0..512 {
        app.departures
            .push(Departure {
                at: BASE + 1 + i * 20,
                group: 1,
                pick: 0,
                conditional: i % 2 == 0,
                headsign: (i % 32) as u8,
            })
            .unwrap();
    }
    (c, app)
}
fn image(app: &App) -> [u8; RETAINED_BYTES] {
    let mut image = [0; RETAINED_BYTES];
    battery::save(
        &mut image,
        app,
        42,
        ANCHOR,
        PERIOD,
        Some(BASE * 1_000_000 + 123_456),
        3_600_000_000,
    )
    .unwrap();
    image
}
fn seal(bytes: &mut [u8; RETAINED_BYTES]) {
    bytes[8..12].fill(0);
    let crc = config::crc32(bytes);
    bytes[8..12].copy_from_slice(&crc.to_le_bytes());
}

#[test]
fn full_cache_survives_sleep_with_exact_times_names_and_manual_group() {
    let (c, original) = board();
    let mut bytes = image(&original);
    let mut app = App::default();
    let wake = battery::restore(&mut bytes, &c, 42, ANCHOR + 900_000_000, &mut app).unwrap();
    assert_eq!(wake.utc_micros, Some(BASE * 1_000_000 + 1_800_123_456));
    assert_eq!(wake.refresh_in_us, 1_800_000_000);
    assert_eq!(
        app.departures.iter().collect::<Vec<_>>(),
        original.departures.iter().collect::<Vec<_>>()
    );
    for i in 0..32 {
        assert_eq!(app.headsigns.get(i), original.headsigns.get(i));
    }
    assert_eq!(app.loaded_at, Some(BASE));
    app.tick(&c, app.clock.unwrap());
    assert_eq!(app.group, 1);
    app.tick(&c, BASE + 7201);
    assert_eq!(app.group, 0);
    assert!(
        bytes
            .windows(c.wifi_password.len())
            .all(|w| w != c.wifi_password.as_bytes())
    );
}

#[test]
fn corrupt_or_mismatched_retention_never_changes_the_app() {
    let (c, original) = board();
    let valid = image(&original);
    let mut app = App {
        loaded_at: Some(77),
        clock: Some(88),
        group: 0,
        ..App::default()
    };
    for at in [0, 8, 12, 16, 24, 32, 40, 48, 64, 66, 70, 72, 80, 4000] {
        let mut bad = valid;
        bad[at] ^= 1;
        assert!(battery::restore(&mut bad, &c, 42, ANCHOR + 1, &mut app).is_err());
        assert_eq!(
            (app.loaded_at, app.clock, app.group, app.departures.len()),
            (Some(77), Some(88), 0, 0)
        );
    }
    assert!(battery::restore(&mut valid.clone(), &c, 43, ANCHOR, &mut app).is_err());
    assert!(battery::restore(&mut valid.clone(), &c, 42, ANCHOR - 1, &mut app).is_err());
    // Valid CRC still must not allow malformed lengths, selections or headsigns.
    for (at, value) in [
        (64, 4),
        (65, 255),
        (66, 255),
        (68, 33),
        (70, 0),
        (72, 0),
        (82, 32),
        (83, 255),
        (2128, 255),
    ] {
        let mut bad = valid;
        bad[at] = value;
        if at == 72 {
            bad[72..76].fill(0);
        }
        seal(&mut bad);
        assert!(
            battery::restore(&mut bad, &c, 42, ANCHOR + 1, &mut app).is_err(),
            "accepted byte {at}"
        );
        assert_eq!(
            (app.loaded_at, app.clock, app.group, app.departures.len()),
            (Some(77), Some(88), 0, 0)
        );
    }
}

#[test]
fn repeated_short_sleeps_preserve_subsecond_time_and_deadline() {
    let (c, mut app) = board();
    let mut utc = BASE * 1_000_000 + 123_456;
    let mut ticks = ANCHOR;
    let mut remaining = 1_000_000;
    let mut bytes = [0; RETAINED_BYTES];
    for _ in 0..20 {
        battery::save(&mut bytes, &app, 42, ticks, PERIOD, Some(utc), remaining).unwrap();
        ticks += 50_000; // 100ms, below the former clock's second resolution.
        let wake = battery::restore(&mut bytes, &c, 42, ticks, &mut app).unwrap();
        utc = wake.utc_micros.unwrap();
        remaining = wake.refresh_in_us;
    }
    assert_eq!(utc, BASE * 1_000_000 + 2_123_456);
    assert_eq!(remaining, 0);
}

#[test]
fn expired_times_and_clockless_failures_have_bounded_retries() {
    let (c, original) = board();
    let mut bytes = image(&original);
    let mut app = App::default();
    let wake =
        battery::restore(&mut bytes, &c, 42, ANCHOR + 7 * 3_600_000_000 / 2, &mut app).unwrap();
    assert_eq!(wake.refresh_in_us, 0);
    assert!(app.upcoming().next().is_none());
    let offline = App {
        status: Status::Offline,
        ..App::default()
    };
    battery::save(&mut bytes, &offline, 42, ANCHOR, PERIOD, None, 300_000_000).unwrap();
    let wake = battery::restore(&mut bytes, &c, 42, ANCHOR + 50_000_000, &mut app).unwrap();
    assert!(wake.utc_micros.is_none());
    assert_eq!(wake.refresh_in_us, 200_000_000);
    assert_eq!(app.status, Status::Offline);
    assert!(app.departures.is_empty());
    assert!(app.clock.is_none());
    assert_eq!(battery::next_refresh(100, false), 300_000_100);
    assert_eq!(battery::next_refresh(100, true), 10_800_000_100);
}

#[test]
fn sleep_does_not_discard_edits_or_interrupt_updates_and_touches() {
    let (c, mut app) = board();
    assert!(!battery::can_sleep(&app, false, 29, false));
    assert!(battery::can_sleep(&app, false, 30, false));
    assert!(!battery::can_sleep(&app, true, 999, false));
    assert!(!battery::can_sleep(&app, false, 999, true));
    app.screen = Screen::Walk;
    assert!(!battery::can_sleep(&app, false, 999, false));
    app.screen = Screen::Board;
    app.draft = Some(c.groups[0].picks[0].clone());
    assert!(!battery::can_sleep(&app, false, 999, false));
    app.draft = None;
    app.save_failed = true;
    assert!(!battery::can_sleep(&app, false, 999, false));
}

#[test]
fn legacy_settings_default_to_always_on_and_battery_roundtrips() {
    let mut c = settings();
    assert!(!c.battery);
    c.battery = true;
    let mut image = [0; config::IMAGE_SIZE];
    config::encode(&c, 3, &mut image).unwrap();
    let (restored, generation) = config::decode(&mut image).unwrap();
    assert!(restored.battery);
    assert_eq!(generation, 3);
    assert!(restored == c);
}

#[test]
fn cached_editor_wake_starts_radio_at_deadline_and_shutdown_touch_is_visible() {
    assert!(!battery::refresh_due(true, false, 99, 100));
    assert!(battery::refresh_due(true, false, 100, 100));
    assert!(!battery::refresh_due(false, false, 101, 100));
    assert!(!battery::refresh_due(true, true, 101, 100));
    let (c, app) = board();
    let mut bytes = image(&app);
    battery::show_on_wake(&mut bytes);
    let wake = battery::restore(&mut bytes, &c, 42, ANCHOR + 500, &mut App::default()).unwrap();
    assert!(wake.show_screen);
    assert_eq!(wake.refresh_in_us, 3_600_000_000 - 1000);
    let ui = battery::wake_ui(Some(&wake), true, false);
    assert!(ui.visible && ui.suppress_touch);
    let mut wake = wake;
    wake.show_screen = false;
    let ui = battery::wake_ui(Some(&wake), true, false);
    assert!(!ui.visible && !ui.suppress_touch);
    let ui = battery::wake_ui(Some(&wake), false, true);
    assert!(ui.visible && ui.suppress_touch);
    assert!(battery::wake_ui(None, true, false).visible);
}
