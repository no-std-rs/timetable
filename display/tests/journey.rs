use gtfs::{Error, Table, csv, zip};
use timetable_display::{
    clock,
    config::Journey,
    feed::Tables,
    journey::{Cache, DevicePlanner, Planner},
};
const NOW: i64 = 1792216800; // 2026-10-17 08:00 Europe/Warsaw.
struct Source<'a>(&'a [u8]);
impl Tables for Source<'_> {
    async fn scan(
        &mut self,
        tables: &[Table],
        expected: Option<&zip::Manifest>,
        workspace: &mut zip::Workspace,
        visit: impl FnMut(csv::Record<'_>) -> Result<(), Error>,
    ) -> Result<zip::Manifest, Error> {
        gtfs::stream::scan(self.0, tables, expected, workspace, visit).await
    }
}
fn settings(depth: u8) -> Journey {
    let json = format!(
        r#"{{"origins":[{{"stop":"A","stop_name":"Maków Polnych","stop_code":"A01","walk":6}},{{"stop":"B","stop_name":"Further stop","stop_code":"B01","walk":12}}],"targets":[{{"stop":"Z","stop_name":"Bolechowo-Os./Szkoła","stop_code":"Z01","walk":0}}],"transfer_minutes":3,"max_transfers":{depth}}}"#
    );
    serde_json_core::from_slice_escaped(json.as_bytes(), &mut [0; 256])
        .unwrap()
        .0
}
struct Planned {
    cache: Cache,
    labels: timetable_display::model::Headings,
}
impl core::ops::Deref for Planned {
    type Target = Cache;
    fn deref(&self) -> &Cache {
        &self.cache
    }
}
impl Planned {
    fn line(&self, index: u8) -> &str {
        self.labels.get(index).unwrap()
    }
    fn stop(&self, index: u8) -> &str {
        self.labels.get(index).unwrap()
    }
}
fn query(bytes: &[u8], config: &Journey) -> Planned {
    query_at(bytes, config, NOW, false)
}
fn query_at(bytes: &[u8], config: &Journey, now: i64, device: bool) -> Planned {
    // Test both actual resource profiles, not just the larger Android search.
    macro_rules! run {
        ($planner:expr, $cache:expr, $labels:expr) => {
            futures::executor::block_on($planner.query(
                &mut Source(bytes),
                &mut zip::Workspace::new(),
                config,
                clock::Zone::WARSAW,
                now,
                7,
                0,
                $cache,
                $labels,
                None,
            ))
            .unwrap()
        };
    }
    let mut labels = timetable_display::model::Headings::default();
    let mut cache = Cache::default();
    cache.reset(now);
    if device {
        let mut planner = DevicePlanner::new("20261017".parse().unwrap());
        run!(planner, &mut cache, &mut labels);
    } else {
        let mut planner = Planner::new("20261017".parse().unwrap());
        run!(planner, &mut cache, &mut labels);
    }
    Planned { cache, labels }
}
#[test]
fn progressively_checks_direct_one_transfer_and_two_transfers() {
    for (depth, minutes, legs) in [(0, 35, 1), (1, 30, 2), (2, 29, 3)] {
        let cache = query(include_bytes!("fixtures/journey.zip"), &settings(depth));
        let best = cache.best(0, NOW).unwrap();
        assert_eq!(best.arrival, minutes * 60);
        assert_eq!(best.len, legs);
        if depth != 0 {
            assert_eq!(cache.line(best.legs[0].line), "911");
        }
    }
}
#[test]
fn a_longer_walk_can_beat_every_connecting_bus() {
    let cache = query(include_bytes!("fixtures/journey-direct.zip"), &settings(2));
    let best = cache.best(0, NOW).unwrap();
    assert_eq!(
        (best.origin, best.walk, best.len, best.arrival),
        (1, 12, 1, 28 * 60)
    );
    assert_eq!(cache.stop(best.legs[0].to), "Bolechowo-Os./Szkoła");
}
#[test]
fn rejects_tight_transfers_and_respects_exact_boundary_and_feed_minimum() {
    let mut config = settings(1);
    let cache = query(include_bytes!("fixtures/journey.zip"), &config);
    assert_eq!(cache.best(0, NOW).unwrap().arrival, 30 * 60); // Exactly 3m is allowed.
    config.transfer_minutes = 2;
    assert_eq!(
        query(include_bytes!("fixtures/journey.zip"), &config)
            .best(0, NOW)
            .unwrap()
            .arrival,
        25 * 60
    );
    config.transfer_minutes = 3;
    assert_eq!(
        query(include_bytes!("fixtures/journey-minimum.zip"), &config)
            .best(0, NOW)
            .unwrap()
            .arrival,
        35 * 60
    );
    assert_eq!(
        query(include_bytes!("fixtures/journey-forbidden.zip"), &config)
            .best(0, NOW)
            .unwrap()
            .arrival,
        35 * 60
    );
}
#[test]
fn keeps_future_options_and_never_suggests_an_uncatchable_first_bus() {
    let cache = query(include_bytes!("fixtures/journey.zip"), &settings(1));
    assert_eq!(cache.best(0, NOW + 4 * 60).unwrap().arrival, 60 * 60);
    assert!(cache.best(0, NOW - 1).is_none());
    assert!(cache.best(0, NOW + 6 * 3600 + 1).is_none());
}
#[test]
fn handles_unordered_stop_calls_and_multiple_targets_without_live_feeds() {
    let ordinary = query(include_bytes!("fixtures/journey.zip"), &settings(2));
    let shuffled = query(
        include_bytes!("fixtures/journey-shuffled.zip"),
        &settings(2),
    );
    assert_eq!(
        ordinary.best(0, NOW).unwrap().arrival,
        shuffled.best(0, NOW).unwrap().arrival
    );
    let mut config = settings(2);
    let mut target = config.targets[0].clone();
    target.stop = "W".try_into().unwrap();
    target.walk = 0;
    config.targets.push(target).unwrap();
    let cache = query(include_bytes!("fixtures/journey.zip"), &config);
    assert_eq!(
        (
            cache.best(0, NOW).unwrap().target,
            cache.best(0, NOW).unwrap().arrival
        ),
        (1, 27 * 60)
    );
    assert!(
        query(include_bytes!("fixtures/journey-no-service.zip"), &config)
            .best(0, NOW)
            .is_none()
    );
}

#[test]
fn same_station_platform_changes_work_but_station_prohibitions_win() {
    let config = settings(1);
    let connected = query(include_bytes!("fixtures/journey-station.zip"), &config);
    assert_eq!(connected.best(0, NOW).unwrap().arrival, 30 * 60);
    let forbidden = query(
        include_bytes!("fixtures/journey-station-forbidden.zip"),
        &config,
    );
    assert_eq!(forbidden.best(0, NOW).unwrap().arrival, 35 * 60);
}

#[test]
fn esp32_resource_profile_matches_android_at_every_device_depth() {
    for depth in 0..=2 {
        for bytes in [
            include_bytes!("fixtures/journey.zip").as_slice(),
            include_bytes!("fixtures/journey-station.zip").as_slice(),
        ] {
            let android = query(bytes, &settings(depth));
            let esp = query_at(bytes, &settings(depth), NOW, true);
            assert_eq!(android.paths, esp.paths);
        }
    }
    assert!(core::mem::size_of::<DevicePlanner>() < 22 * 1024);
}

#[test]
fn previous_service_day_after_24h_and_boarding_restrictions() {
    for device in [false, true] {
        let midnight = NOW + 16 * 3600;
        let night = query_at(
            include_bytes!("fixtures/journey-night.zip"),
            &settings(1),
            midnight,
            device,
        );
        assert_eq!(night.best(0, midnight).unwrap().arrival, 30 * 60);
        for bytes in [
            include_bytes!("fixtures/journey-no-pickup.zip").as_slice(),
            include_bytes!("fixtures/journey-no-dropoff.zip").as_slice(),
        ] {
            let result = query_at(bytes, &settings(1), NOW, device);
            assert_eq!(result.best(0, NOW).unwrap().arrival, 35 * 60);
        }
    }
}

#[test]
fn snapshot_preserves_future_connections_and_rejects_invalid_references() {
    let planned = query(include_bytes!("fixtures/journey.zip"), &settings(1));
    let mut config = timetable_display::Config::empty();
    config
        .groups
        .push(timetable_display::config::Group {
            name: "School".try_into().unwrap(),
            picks: Default::default(),
            days: 127,
            start: 0,
            end: 1440,
            journey: Some(settings(1)),
        })
        .unwrap();
    let mut bytes = [0; 4096];
    let len = planned.encode(&mut bytes).unwrap();
    let restored = Cache::decode(&bytes[..len], &config, &planned.labels).unwrap();
    assert_eq!(planned.paths, restored.paths);
    assert_eq!(restored.best(0, NOW + 4 * 60).unwrap().arrival, 60 * 60);
    bytes[38] = 255; // First leg's line label.
    assert!(Cache::decode(&bytes[..len], &config, &planned.labels).is_err());
    assert!(Cache::decode(&bytes[..len - 1], &config, &planned.labels).is_err());
}

#[test]
fn esp32_heap_envelope_includes_radio_and_persistent_staging() {
    // Same conservative radio allowance as the existing constrained probe.
    // This is an admission model, not a physical radio peak measurement.
    let needed = 34320
        + core::mem::size_of::<timetable_display::Config>()
        + core::mem::size_of::<Cache>()
        + core::mem::size_of::<DevicePlanner>()
        + 512;
    println!(
        "radio + settings + result + planner + allocation allowance = {needed} B; planner {} B",
        core::mem::size_of::<DevicePlanner>()
    );
    assert!(needed <= 65536);
}
