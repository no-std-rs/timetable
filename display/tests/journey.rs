use gtfs::{Error, Table, csv, zip};
use timetable_display::{
    clock,
    config::Journey,
    feed::Tables,
    journey::{
        Cache, DevicePlanner, HORIZON, LEG_REQUEST_BOARD, LIMIT_TRANSFERS, LIMIT_WINDOW, Planner,
        Search,
    },
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
            end: 0,
            journey: Some(settings(1)),
        })
        .unwrap();
    let mut bytes = [0; 4096];
    let len = planned.encode(&mut bytes).unwrap();
    let restored = Cache::decode(&bytes[..len], &config, &planned.labels).unwrap();
    assert_eq!(planned.paths, restored.paths);
    assert_eq!(restored.best(0, NOW + 4 * 60).unwrap().arrival, 60 * 60);
    let mut old = bytes;
    old[..8].copy_from_slice(b"TTJNY001");
    assert!(Cache::decode(&old[..len], &config, &planned.labels).is_err());
    bytes[48] = 255; // First leg's line label, after the 38-byte header and path row.
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

#[test]
fn dwell_calls_can_cross_both_search_window_boundaries() {
    for device in [false, true] {
        let result = query_at(
            include_bytes!("fixtures/journey-boundaries.zip"),
            &settings(0),
            NOW,
            device,
        );
        let best = result.best(0, NOW).unwrap();
        assert_eq!((best.legs[0].departure, best.arrival), (7 * 60, 359 * 60));
    }
}
#[test]
fn request_stops_are_ordinary_boarding_and_alighting_marked_on_the_leg() {
    use timetable_display::journey::{LEG_REQUEST_ALIGHT, LEG_REQUEST_BOARD};
    for device in [false, true] {
        // Poznań marks request stops ("na żądanie") as pickup/drop-off type 3;
        // type 2 needs an arrangement. Both are planned and marked.
        let board = query_at(
            include_bytes!("fixtures/journey-conditional.zip"),
            &settings(1),
            NOW,
            device,
        );
        let best = board.best(0, NOW).unwrap();
        assert_eq!(best.arrival, 30 * 60);
        assert_eq!(best.legs[0].flags, LEG_REQUEST_BOARD);
        let alight = query_at(
            include_bytes!("fixtures/journey-conditional-alight.zip"),
            &settings(1),
            NOW,
            device,
        );
        let best = alight.best(0, NOW).unwrap();
        assert_eq!(best.arrival, 30 * 60);
        assert_eq!(best.legs[best.len as usize - 1].flags, LEG_REQUEST_ALIGHT);
        let plain = query_at(
            include_bytes!("fixtures/journey.zip"),
            &settings(1),
            NOW,
            device,
        );
        assert!(
            plain
                .paths
                .iter()
                .all(|p| p.legs.iter().all(|l| l.flags == 0))
        );
    }
}

#[test]
fn relevant_unsupported_rules_fail_explicitly_but_remote_rules_do_not() {
    assert_eq!(
        query(
            include_bytes!("fixtures/journey-conditional-through.zip"),
            &settings(0)
        )
        .best(0, NOW)
        .unwrap()
        .arrival,
        35 * 60
    );
    assert_eq!(
        query(
            include_bytes!("fixtures/journey-remote-rule.zip"),
            &settings(1)
        )
        .best(0, NOW)
        .unwrap()
        .arrival,
        30 * 60
    );
    for bytes in [
        include_bytes!("fixtures/journey-scoped-rule.zip").as_slice(),
        include_bytes!("fixtures/journey-inseat-rule.zip").as_slice(),
    ] {
        let mut planner = DevicePlanner::new("20261017".parse().unwrap());
        let mut output = Cache::default();
        output.reset(NOW);
        let error = futures::executor::block_on(planner.query(
            &mut Source(bytes),
            &mut zip::Workspace::new(),
            &settings(1),
            clock::Zone::WARSAW,
            NOW,
            7,
            0,
            &mut output,
            &mut Default::default(),
            None,
        ))
        .unwrap_err();
        assert!(matches!(error, Error::InvalidField(_)), "{error:?}");
        assert!(output.paths.is_empty());
    }
}
#[test]
fn empty_and_expired_journeys_can_refresh_by_touch_on_every_display() {
    use timetable_display::{
        App, Config, Screen,
        catalog::Catalog,
        model::Status,
        ui::{self, Action},
    };
    let mut config: Config = serde_json_core::from_slice(include_bytes!("settings.json"))
        .unwrap()
        .0;
    config.groups[0].journey = Some(settings(1));
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    for expired in [false, true] {
        for layout in 0..3 {
            let mut app = App {
                clock: Some(NOW),
                status: Status::Ready,
                ..Default::default()
            };
            if expired {
                app.journeys = query(include_bytes!("fixtures/journey.zip"), &settings(1)).cache;
                app.clock = Some(NOW + 6 * 3600 + 1);
            }
            assert!(app.can_refresh(&config));
            let action = match layout {
                0 => ui::tap(&mut app, &mut config, &catalog, 120, 37, false),
                1 => ui::tap_portrait(&mut app, &mut config, &catalog, 86, 37, false),
                _ => ui::tap_landscape(&mut app, &mut config, &catalog, 160, 37, false),
            };
            assert_eq!(action, Action::Refresh);
            assert_eq!(app.screen, Screen::Board);
            assert_eq!(app.status, Status::Loading);
        }
    }
    for status in [Status::Loading, Status::Connecting, Status::Syncing] {
        let app = App {
            clock: Some(NOW),
            status,
            ..Default::default()
        };
        assert!(!app.can_refresh(&config));
    }
    config.groups[0].journey = None;
    assert!(
        !App {
            clock: Some(NOW),
            status: Status::Ready,
            ..Default::default()
        }
        .can_refresh(&config)
    );
}

#[test]
fn three_leg_landscape_keeps_destination_text_out_of_the_footer() {
    use core::convert::Infallible;
    use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
    struct Footer {
        detail: usize,
        summary: usize,
    }
    impl OriginDimensions for Footer {
        fn size(&self) -> Size {
            Size::new(320, 172)
        }
    }
    impl DrawTarget for Footer {
        type Color = Rgb565;
        type Error = Infallible;
        fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
            &mut self,
            pixels: I,
        ) -> Result<(), Infallible> {
            for Pixel(point, color) in pixels {
                if point.y >= 154 && point.y < 172 {
                    self.detail += usize::from(color == Rgb565::new(15, 30, 14));
                    self.summary += usize::from(color == timetable_display::render::GOLD);
                }
            }
            Ok(())
        }
    }
    let planned = query(include_bytes!("fixtures/journey.zip"), &settings(2));
    let mut config: timetable_display::Config =
        serde_json_core::from_slice(include_bytes!("settings.json"))
            .unwrap()
            .0;
    config.groups[0].journey = Some(settings(2));
    let app = timetable_display::App {
        journeys: planned.cache,
        headsigns: planned.labels,
        clock: Some(NOW),
        loaded_at: Some(NOW),
        status: timetable_display::model::Status::Ready,
        ..Default::default()
    };
    let catalog =
        timetable_display::catalog::Catalog::new(include_bytes!("../../catalog/catalog.bin"))
            .unwrap();
    let mut footer = Footer {
        detail: 0,
        summary: 0,
    };
    timetable_display::render::landscape::draw_animated(&mut footer, &app, &config, &catalog, 0)
        .unwrap();
    assert_eq!(footer.detail, 0, "leg detail intrudes into footer area");
    assert!(footer.summary > 0, "connection summary must remain visible");
}

#[test]
fn timed_transfer_guarantees_do_not_waive_the_users_minimum_slack() {
    let mut config = settings(1);
    assert_eq!(
        query(include_bytes!("fixtures/journey-timed.zip"), &config)
            .best(0, NOW)
            .unwrap()
            .arrival,
        30 * 60
    );
    config.transfer_minutes = 2;
    assert_eq!(
        query(include_bytes!("fixtures/journey-timed.zip"), &config)
            .best(0, NOW)
            .unwrap()
            .arrival,
        25 * 60
    );
}

#[test]
fn named_endpoints_include_all_platforms_without_consuming_stop_slots() {
    for device in [false, true] {
        for bytes in [
            include_bytes!("fixtures/journey-places.zip").as_slice(),
            include_bytes!("fixtures/journey-places-shuffled.zip").as_slice(),
        ] {
            let mut config = settings(1);
            config.origins.truncate(1);
            let result = query_at(bytes, &config, NOW, device);
            let best = result.best(0, NOW).unwrap();
            assert_eq!((best.arrival, best.len), (22 * 60, 1));
            assert_eq!(result.line(best.legs[0].line), "Platform");
            assert!(result.stop(best.legs[0].from).ends_with(" · A02"));
            assert!(result.stop(best.legs[0].to).ends_with(" · Z02"));
            assert_eq!((best.origin, best.target, best.walk), (0, 0, 6));
            // The same named origin with a longer walk misses the new service.
            config.origins[0].walk = 10;
            let slow = query_at(bytes, &config, NOW, device);
            assert!(slow.best(0, NOW).unwrap().arrival > best.arrival);
            // Accent-insensitive search must not conflate two actual places.
            config.origins[0].stop = "U".try_into().unwrap();
            assert!(query_at(bytes, &config, NOW, device).best(0, NOW).is_none());
        }
    }
}

#[test]
fn same_named_interchange_platforms_connect_without_station_metadata() {
    for device in [false, true] {
        for depth in [1, 2] {
            for (bytes, minutes) in [
                (
                    include_bytes!("fixtures/journey-place-transfer.zip").as_slice(),
                    if depth == 1 { 30 } else { 29 },
                ),
                (
                    include_bytes!("fixtures/journey-place-transfer-forbidden.zip").as_slice(),
                    if depth == 1 { 35 } else { 29 },
                ),
                (
                    include_bytes!("fixtures/journey-place-transfer-minimum.zip").as_slice(),
                    if depth == 1 { 35 } else { 29 },
                ),
            ] {
                let result = query_at(bytes, &settings(depth), NOW, device);
                assert_eq!(result.best(0, NOW).unwrap().arrival, minutes * 60);
            }
        }
    }
}

#[test]
fn deeper_search_discovers_middle_ride_on_other_same_name_platforms() {
    for device in [false, true] {
        for (depth, arrival) in [(1, 35 * 60), (2, 29 * 60)] {
            let result = query_at(
                include_bytes!("fixtures/journey-place-middle.zip"),
                &settings(depth),
                NOW,
                device,
            );
            assert_eq!(result.best(0, NOW).unwrap().arrival, arrival);
        }
    }
}

#[test]
fn repeated_names_respect_geography_and_declared_station_identity() {
    for device in [false, true] {
        for (bytes, minutes) in [
            (
                include_bytes!("fixtures/journey-places-far.zip").as_slice(),
                30,
            ),
            (
                include_bytes!("fixtures/journey-places-unlocated.zip").as_slice(),
                30,
            ),
            (
                include_bytes!("fixtures/journey-places-parents.zip").as_slice(),
                30,
            ),
            (
                include_bytes!("fixtures/journey-places-shared-parent.zip").as_slice(),
                22,
            ),
            (
                include_bytes!("fixtures/journey-place-transfer-far.zip").as_slice(),
                35,
            ),
        ] {
            let result = query_at(bytes, &settings(1), NOW, device);
            assert_eq!(result.best(0, NOW).unwrap().arrival, minutes * 60);
        }
    }
}
#[test]
fn middle_platform_rules_apply_to_changes_rather_than_staying_on_the_bus() {
    for device in [false, true] {
        for (bytes, minutes) in [
            (
                include_bytes!("fixtures/journey-place-middle-blocked.zip").as_slice(),
                35,
            ),
            (
                include_bytes!("fixtures/journey-place-middle-inseat-stops.zip").as_slice(),
                29,
            ),
        ] {
            // Each change has an endpoint-trip side. The XP->YP rule names two
            // new middle stops; the passenger stays on one bus between them.
            let result = query_at(bytes, &settings(2), NOW, device);
            assert_eq!(result.best(0, NOW).unwrap().arrival, minutes * 60);
        }
    }
}

#[test]
fn overlapping_place_halos_use_one_partition_for_every_endpoint_and_interchange() {
    for device in [false, true] {
        for bytes in [
            include_bytes!("fixtures/journey-places-overlap.zip").as_slice(),
            include_bytes!("fixtures/journey-places-overlap-shuffled.zip").as_slice(),
        ] {
            let mut config = settings(1);
            config.origins[0].stop = "A3".try_into().unwrap();
            // A/A2 is one place. A3 cannot gain A2's faster service merely
            // because A2 is also inside A3's independently measured halo.
            assert_eq!(
                query_at(bytes, &config, NOW, device)
                    .best(0, NOW)
                    .unwrap()
                    .arrival,
                35 * 60
            );
            config.origins[0].stop = "A2".try_into().unwrap();
            assert_eq!(
                query_at(bytes, &config, NOW, device)
                    .best(0, NOW)
                    .unwrap()
                    .arrival,
                22 * 60
            );
        }
        // X is unserved, but defines XP's global representative. It must not
        // disappear from the partition when only XQ occurs on an endpoint trip.
        assert_eq!(
            query_at(
                include_bytes!("fixtures/journey-place-middle-overlap.zip"),
                &settings(2),
                NOW,
                device
            )
            .best(0, NOW)
            .unwrap()
            .arrival,
            35 * 60
        );
    }
}

fn city(depth: u8) -> Journey {
    let json = format!(
        r#"{{"origins":[{{"stop":"A-1","walk":6}},{{"stop":"B-1","walk":12}}],"targets":[{{"stop":"Z-1","walk":0}},{{"stop":"W-1","walk":2}}],"transfer_minutes":3,"max_transfers":{depth}}}"#
    );
    serde_json_core::from_slice_escaped(json.as_bytes(), &mut [0; 256])
        .unwrap()
        .0
}
fn planned_with<
    P: core::ops::DerefMut<Target = Search<C, S, T, M, V, R>>,
    const C: usize,
    const S: usize,
    const T: usize,
    const M: usize,
    const V: usize,
    const R: usize,
>(
    mut planner: P,
    bytes: &[u8],
    config: &Journey,
) -> Planned {
    let mut labels = timetable_display::model::Headings::default();
    let mut cache = Cache::default();
    cache.reset(NOW);
    futures::executor::block_on(planner.query(
        &mut Source(bytes),
        &mut zip::Workspace::new(),
        config,
        clock::Zone::WARSAW,
        NOW,
        7,
        0,
        &mut cache,
        &mut labels,
        None,
    ))
    .unwrap();
    Planned { cache, labels }
}
fn date() -> gtfs::Date {
    "20261017".parse().unwrap()
}
fn assert_nondominated(cache: &Cache) {
    for a in &cache.paths {
        for b in &cache.paths {
            if a != b {
                let dominates = a.leave >= b.leave
                    && a.arrival <= b.arrival
                    && (a.arrival < b.arrival || (a.len, a.walk) <= (b.len, b.walk));
                assert!(!dominates, "{a:?} dominates {b:?}");
            }
        }
    }
}

#[test]
fn esp32_plans_a_dense_city_with_request_stops_without_shortening() {
    // Inactive weekday trips, ~140 platforms and thousands of calls exceeded the
    // former device bounds. Only places both sides can use are now stored.
    let bytes = include_bytes!("fixtures/journey-city.zip");
    let android = query_at(bytes, &city(1), NOW, false);
    let esp = query_at(bytes, &city(1), NOW, true);
    for result in [&android, &esp] {
        assert_eq!(
            (result.windows[0], result.limits[0]),
            (HORIZON as u16, 0),
            "complete six-hour profile"
        );
        let best = result.best(0, NOW).unwrap();
        assert_eq!(
            (best.arrival, best.len, best.origin, best.target, best.walk),
            (35 * 60, 1, 1, 1, 12)
        );
        assert_eq!(result.line(best.legs[0].line), "348");
        // Missing it: walk to Maków Polnych, request the 911, change at Kresowa.
        let then = result.next_after(0, NOW, best).unwrap();
        assert_eq!(
            (then.leave, then.arrival, then.len, then.origin),
            (3 * 60, 35 * 60, 2, 0)
        );
        assert_eq!(result.line(then.legs[0].line), "911");
        assert_eq!(then.legs[0].flags, LEG_REQUEST_BOARD);
        assert!(result.stop(then.legs[0].to).starts_with("Kresowa"));
        assert_eq!(result.best(0, NOW + 2 * 60), Some(then));
        assert_nondominated(result);
    }
    assert_eq!(android.paths, esp.paths);
}

#[test]
fn any_target_counts_so_continuing_past_an_earlier_target_is_never_kept() {
    let mut config = settings(2);
    let mut target = config.targets[0].clone();
    target.stop = "W".try_into().unwrap();
    config.targets.push(target).unwrap();
    for device in [false, true] {
        let result = query_at(include_bytes!("fixtures/journey.zip"), &config, NOW, device);
        assert_nondominated(&result);
        // The 08:15 direct bus reaches W at 08:27 and Z at 08:35; only W remains.
        assert!(
            !result
                .paths
                .iter()
                .any(|p| p.legs[p.len as usize - 1].departure == 15 * 60 && p.target == 0)
        );
        assert!(
            result
                .paths
                .iter()
                .any(|p| p.target == 1 && p.arrival == 27 * 60)
        );
    }
}

#[test]
fn a_full_store_shortens_the_window_and_stays_exact_before_its_end() {
    let bytes = include_bytes!("fixtures/journey-city.zip");
    let full = query(bytes, &city(1));
    let short = planned_with(
        Box::new(Search::<300, 128, 512, 128, 64, 32>::new(date())),
        bytes,
        &city(1),
    );
    let window = short.windows[0];
    assert_ne!(short.limits[0] & LIMIT_WINDOW, 0);
    assert!((3600..HORIZON as u16).contains(&window), "{window}");
    assert!(short.paths.iter().all(|p| p.arrival <= window));
    for minute in (0..i64::from(window) / 60).step_by(3) {
        let at = NOW + minute * 60;
        match full.best(0, at) {
            Some(best) if best.arrival <= window => {
                assert_eq!(short.best(0, at).map(|p| p.arrival), Some(best.arrival));
            }
            _ => assert!(short.best(0, at).is_none(), "{minute}"),
        }
    }
}

#[test]
fn transfer_places_that_do_not_fit_keep_exact_direct_journeys() {
    let bytes = include_bytes!("fixtures/journey-city.zip");
    let direct = planned_with(
        Box::new(Search::<
            { timetable_display::journey::DEVICE_CALLS },
            12,
            512,
            128,
            64,
            32,
        >::new(date())),
        bytes,
        &city(1),
    );
    assert_ne!(direct.limits[0] & LIMIT_TRANSFERS, 0);
    assert!(direct.paths.iter().all(|p| p.len == 1));
    assert_eq!(direct.best(0, NOW).unwrap().arrival, 35 * 60);
    // Without the 911 change, the next direct 348 leaves at 08:21.
    let later = direct.best(0, NOW + 2 * 60).unwrap();
    assert_eq!((later.leave, later.arrival), (21 * 60, 55 * 60));
}

#[test]
fn two_transfer_search_reports_its_limit_and_keeps_one_transfer_results() {
    let bytes = include_bytes!("fixtures/journey-city.zip");
    for device in [false, true] {
        let deep = query_at(bytes, &city(2), NOW, device);
        let one = query_at(bytes, &city(1), NOW, device);
        // Every change place of this dense city does not fit the bounded
        // middle-ride store: the search says so instead of failing.
        assert_eq!(deep.limits[0], LIMIT_TRANSFERS);
        assert_eq!(deep.paths, one.paths);
    }
}

#[test]
fn snapshot_keeps_windows_limits_and_request_stops() {
    let bytes = include_bytes!("fixtures/journey-city.zip");
    let short = planned_with(
        Box::new(Search::<300, 128, 512, 128, 64, 32>::new(date())),
        bytes,
        &city(1),
    );
    let mut config = timetable_display::Config::empty();
    config
        .groups
        .push(timetable_display::config::Group {
            name: "School".try_into().unwrap(),
            picks: Default::default(),
            days: 127,
            start: 0,
            end: 0,
            journey: Some(city(1)),
        })
        .unwrap();
    let mut encoded = [0; 4096];
    let len = short.encode(&mut encoded).unwrap();
    let restored = Cache::decode(&encoded[..len], &config, &short.labels).unwrap();
    assert_eq!(restored.paths, short.paths);
    assert_eq!(
        (restored.windows, restored.limits),
        (short.windows, short.limits)
    );
    assert!(restored.paths.iter().any(|p| p.legs[0].flags != 0));
    assert_eq!(restored.until_for(0), NOW + i64::from(short.windows[0]));
}

#[test]
fn stops_added_from_the_route_view_give_the_device_the_direct_bus() {
    let bytes = include_bytes!("fixtures/journey-walkable.zip");
    let config = |json: &str| -> Journey {
        serde_json_core::from_slice_escaped(json.as_bytes(), &mut [0; 256])
            .unwrap()
            .0
    };
    let nearer = config(
        r#"{"origins":[{"stop":"A","walk":6}],"targets":[{"stop":"S","walk":0}],"transfer_minutes":3,"max_transfers":1}"#,
    );
    // After tapping UAM Wydział Geografii and Wojska Polskiego in the browser.
    let added = config(
        r#"{"origins":[{"stop":"A","walk":6},{"stop":"G","walk":12}],"targets":[{"stop":"S","walk":0},{"stop":"W","walk":2}],"transfer_minutes":3,"max_transfers":1}"#,
    );
    for device in [false, true] {
        let before = query_at(bytes, &nearer, NOW, device);
        let best = before.best(0, NOW).unwrap();
        assert_eq!(
            (best.len, best.arrival, best.legs[0].flags),
            (2, 50 * 60, LEG_REQUEST_BOARD)
        );
        let after = query_at(bytes, &added, NOW, device);
        let best = after.best(0, NOW).unwrap();
        assert_eq!(
            (best.len, best.origin, best.target, best.arrival),
            (1, 1, 1, 37 * 60)
        );
        assert_eq!(after.line(best.legs[0].line), "348");
    }
}
#[test]
fn a_short_turn_can_feed_a_branch_trip_of_the_same_route() {
    for device in [false, true] {
        let cache = query_at(
            include_bytes!("fixtures/journey-branch.zip"),
            &settings(1),
            NOW,
            device,
        );
        let best = cache.best(0, NOW).unwrap();
        assert_eq!((best.len, best.arrival), (2, 35 * 60));
        assert_eq!(
            (cache.line(best.legs[0].line), cache.line(best.legs[1].line)),
            ("348", "348")
        );
    }
}
