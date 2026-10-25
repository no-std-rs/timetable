use timetable_display::{App, Departure};
use timetable_tablet::{DisplayList, Rotation, Settings, State, board, feed, motion, snapshot};
const SETTINGS: &[u8] = include_bytes!("fixtures/settings.json");
const NOW: i64 = 1792216800; // 2026-10-17 06:00 UTC / 08:00 fixed fixture zone.

#[test]
fn settings_reject_bad_versions_and_unknown_fields() {
    let value = Settings::decode(SETTINGS).unwrap();
    assert_eq!(value.groups.len(), 2);
    assert!(Settings::decode(b"{}").is_err());
    let changed = String::from_utf8(SETTINGS.to_vec())
        .unwrap()
        .replace("\"version\":1", "\"version\":2");
    assert!(Settings::decode(changed.as_bytes()).is_err());
    let changed = String::from_utf8(SETTINGS.to_vec()).unwrap().replacen(
        '{',
        "{\"wifi_password\":\"secret\",",
        1,
    );
    assert!(Settings::decode(changed.as_bytes()).is_err());
}
#[test]
fn rotation_accepts_old_settings_and_all_display_directions() {
    assert_eq!(
        Settings::decode(SETTINGS).unwrap().rotation,
        Rotation::System
    );
    let original = std::str::from_utf8(SETTINGS).unwrap();
    for (name, expected) in [
        ("system", Rotation::System),
        ("portrait", Rotation::Portrait),
        ("landscape", Rotation::Landscape),
        ("reverse_portrait", Rotation::ReversePortrait),
        ("reverse_landscape", Rotation::ReverseLandscape),
    ] {
        let changed = original.replacen('{', &format!("{{\"rotation\":\"{name}\","), 1);
        assert_eq!(
            Settings::decode(changed.as_bytes()).unwrap().rotation,
            expected
        );
    }
    let invalid = original.replacen('{', "{\"rotation\":\"sideways\",", 1);
    assert!(Settings::decode(invalid.as_bytes()).is_err());
}
fn populated() -> App {
    let mut app = App::default();
    app.departures.reset(NOW);
    app.loaded_at = Some(NOW);
    let heading = app.headsigns.insert("City centre").unwrap();
    for n in 1..=20 {
        app.departures
            .push(Departure {
                at: NOW + n * 120,
                group: 0,
                pick: 0,
                conditional: false,
                headsign: heading,
            })
            .unwrap();
    }
    app.freshness.checked_at = Some(NOW - 60);
    app
}
#[test]
fn snapshot_crc_settings_identity_and_atomic_restore() {
    let app = populated();
    let mut bytes = [0; 16384];
    let n = snapshot::encode(SETTINGS, &app, &mut bytes).unwrap();
    let mut restored = App::default();
    snapshot::restore(SETTINGS, &bytes[..n], &mut restored).unwrap();
    assert_eq!(restored.departures.len(), 20);
    assert_eq!(restored.freshness.checked_at, Some(NOW - 60));
    bytes[50] ^= 1;
    assert!(snapshot::restore(SETTINGS, &bytes[..n], &mut restored).is_err());
    assert_eq!(restored.departures.len(), 20);
    bytes[50] ^= 1;
    let other = [SETTINGS, b" "].concat();
    assert!(snapshot::restore(&other, &bytes[..n], &mut restored).is_err());
    assert_eq!(restored.departures.len(), 20);
}
#[test]
fn local_query_uses_real_gtfs_and_preserves_source_check_time() {
    let mut bytes = [0; 16384];
    let n = feed::immediate(feed::query(
        SETTINGS,
        include_bytes!("../../gtfs/tests/fixtures/gtfs.zip"),
        NOW,
        NOW - 3600,
        &mut bytes,
    ))
    .unwrap()
    .unwrap();
    let mut app = App::default();
    snapshot::restore(SETTINGS, &bytes[..n], &mut app).unwrap();
    assert!(!app.departures.is_empty());
    assert_eq!(app.freshness.checked_at, Some(NOW - 3600));
    let mut broken = include_bytes!("../../gtfs/tests/fixtures/gtfs.zip").to_vec();
    broken.truncate(broken.len() - 9);
    assert!(
        feed::immediate(feed::query(SETTINGS, &broken, NOW, NOW, &mut bytes))
            .unwrap()
            .is_err()
    );
}
#[derive(Default)]
struct Count {
    texts: usize,
}
impl timetable_tablet::Painter for Count {
    type Error = std::convert::Infallible;
    fn rect(&mut self, _: i32, _: i32, _: i32, _: i32, _: i32, _: u32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn text(
        &mut self,
        _: &str,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: u32,
        _: u32,
    ) -> Result<(), Self::Error> {
        self.texts += 1;
        Ok(())
    }
}
#[test]
fn layouts_use_available_space_and_share_group_hit_regions() {
    let settings = Settings::decode(SETTINGS).unwrap();
    let mut state = State::default();
    let input = |width, height| board::Input {
        width,
        height,
        now: NOW,
        monotonic_ms: 1000,
        battery: 72,
        plugged: true,
        motion_available: false,
        status: 0,
    };
    let mut short = Count::default();
    board::draw(
        &settings,
        &mut populated(),
        &mut state,
        input(600, 600),
        &mut short,
    )
    .unwrap();
    let mut tall = Count::default();
    board::draw(
        &settings,
        &mut populated(),
        &mut state,
        input(600, 1024),
        &mut tall,
    )
    .unwrap();
    assert!(tall.texts > short.texts);
    assert_eq!(
        board::tap(&settings, &mut state, (1024, 600), (800, 110), NOW, 2000),
        0
    );
    assert_eq!(state.group, 1);
    assert_eq!(state.manual_until, NOW + 7200);
    let mut bytes = [0; 32768];
    let mut list = DisplayList::new(&mut bytes).unwrap();
    board::draw(
        &settings,
        &mut populated(),
        &mut state,
        input(1024, 600),
        &mut list,
    )
    .unwrap();
    assert!(list.finish(1, 1) > 24);
    assert_eq!(&bytes[..8], b"TTDRAW01");
}
#[test]
fn camera_motion_is_local_debounced_and_presence_timeout_is_monotonic() {
    let mut history = [0; 800];
    let mut image = [30; 32 * 24];
    assert!(!motion::detect(&mut history, &image, 32, 24).unwrap());
    image.fill(200);
    assert!(!motion::detect(&mut history, &image, 32, 24).unwrap());
    image.fill(30);
    assert!(motion::detect(&mut history, &image, 32, 24).unwrap());
    assert!(motion::detect(&mut history, &image[..4], 32, 24).is_err());
    let mut settings = Settings::decode(SETTINGS).unwrap();
    settings.camera_motion = true;
    let mut state = State::default();
    assert!(!state.dim(&settings, 1000));
    assert!(state.dim(&settings, 121000));
    state.active(122000);
    assert!(!state.dim(&settings, 122001));
    assert!(!state.dim(&settings, 1)); // clock reset, not a permanent dim state.
}
#[derive(Default)]
struct Texts(Vec<String>);
impl timetable_tablet::Painter for Texts {
    type Error = std::convert::Infallible;
    fn rect(&mut self, _: i32, _: i32, _: i32, _: i32, _: i32, _: u32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn text(
        &mut self,
        text: &str,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: u32,
        _: u32,
    ) -> Result<(), Self::Error> {
        self.0.push(text.into());
        Ok(())
    }
}
#[test]
fn journey_footer_never_claims_a_complete_search_after_a_fallback() {
    use timetable_display::journey::{LIMIT_TRANSFERS, Leg, Path};
    let original = std::str::from_utf8(SETTINGS).unwrap();
    let start = original.find("\"picks\":[").unwrap();
    let end = original.find("]},{\"name\":\"Evening\"").unwrap();
    let json = format!(
        "{}\"picks\":[],\"journey\":{{\"origins\":[{{\"stop\":\"A\",\"stop_name\":\"Maków Polnych\",\"stop_code\":\"A01\",\"walk\":6}}],\"targets\":[{{\"stop\":\"Z\",\"stop_name\":\"Bolechowo-Os./Szkoła\",\"stop_code\":\"Z01\",\"walk\":0}}],\"transfer_minutes\":3,\"max_transfers\":2}}{}",
        &original[..start],
        &original[end + 1..]
    );
    let settings = Settings::decode(json.as_bytes()).unwrap();
    for (limits, expected) in [
        (0, "On-device search complete · up to 2 transfers"),
        (
            LIMIT_TRANSFERS,
            "On-device search limited: not every journey with up to 2 transfers checked",
        ),
    ] {
        let mut app = App {
            loaded_at: Some(NOW),
            ..App::default()
        };
        let line = app.headsigns.insert("911").unwrap();
        let stop = app.headsigns.insert("Maków Polnych").unwrap();
        app.journeys.reset(NOW);
        app.journeys.limits[0] = limits;
        let leg = Leg {
            line,
            from: stop,
            to: stop,
            flags: timetable_display::journey::LEG_REQUEST_BOARD,
            departure: 600,
            arrival: 1200,
        };
        app.journeys
            .paths
            .push(Path {
                walk: 6,
                egress: 2,
                len: 1,
                leave: 240,
                arrival: 1200,
                legs: [leg; 3],
                ..Path::default()
            })
            .unwrap();
        let mut texts = Texts::default();
        let input = board::Input {
            width: 1024,
            height: 600,
            now: NOW,
            monotonic_ms: 1000,
            battery: 72,
            plugged: true,
            motion_available: false,
            status: 0,
        };
        board::draw(
            &settings,
            &mut app,
            &mut State::default(),
            input,
            &mut texts,
        )
        .unwrap();
        let footer = texts.0.iter().find(|t| t.starts_with("On-device")).unwrap();
        assert!(footer.starts_with(expected), "{footer}");
        assert_eq!(footer.contains("complete"), limits == 0, "{footer}");
        assert!(
            footer.ends_with("signal the driver or phone the operator"),
            "{footer}"
        );
    }
}
