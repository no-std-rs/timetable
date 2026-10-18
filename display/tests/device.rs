use timetable_display::{
    App, Config, Screen,
    catalog::Catalog,
    clock, config,
    model::{self, Departure},
    ui,
};

fn settings() -> Config {
    serde_json_core::from_slice::<Config>(include_bytes!("settings.json"))
        .unwrap()
        .0
}

#[test]
fn retry_keeps_the_validated_board_and_does_not_save_settings() {
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    for status in [model::Status::Offline, model::Status::Failed] {
        let mut c = settings();
        let original = c.clone();
        let mut app = App {
            status,
            loaded_at: Some(100),
            freshness: timetable_display::freshness::Freshness {
                checked_at: Some(105),
                source_updated_at: Some(90),
            },
            ..App::default()
        };
        app.departures.reset(100);
        app.departures
            .push(Departure {
                at: 200,
                group: 0,
                pick: 0,
                conditional: false,
                headsign: 0,
            })
            .unwrap();
        assert!(app.can_retry());
        assert_eq!(
            ui::tap(&mut app, &mut c, &catalog, 120, 42, false),
            ui::Action::Refresh
        );
        assert!(!app.can_retry());
        assert_eq!(app.screen, Screen::Board);
        assert_eq!(app.loaded_at, Some(100));
        assert_eq!(app.freshness.checked_at, Some(105));
        assert_eq!(app.freshness.source_updated_at, Some(90));
        assert_eq!(app.departures.get(0).unwrap().at, 200);
        assert!(c == original);
        app.status = status;
        assert_eq!(
            ui::tap(&mut app, &mut c, &catalog, 120, 212, true),
            ui::Action::None
        );
        assert_eq!(app.screen, Screen::Picks);
    }
    for (status, save_failed) in [
        (model::Status::Capacity, false),
        (model::Status::Failed, true),
    ] {
        let mut c = settings();
        let mut app = App {
            status,
            save_failed,
            ..App::default()
        };
        assert!(!app.can_retry());
        assert_eq!(
            ui::tap(&mut app, &mut c, &catalog, 120, 212, false),
            ui::Action::None
        );
        assert_eq!(app.screen, Screen::Freshness);
    }
}

#[test]
fn grouped_and_interleaved_trip_lookups_keep_exact_hits_and_misses() {
    futures::executor::block_on(async {
        let catalog = Catalog::new(include_bytes!("fixtures/catalog.bin")).unwrap();
        let stop = catalog.stop(0).unwrap();
        let mut c = settings();
        let p = &mut c.groups[0].picks[0];
        p.route = "R4".try_into().unwrap();
        p.stop = "S1".try_into().unwrap();
        p.stop_code.clear();
        p.stop_name = stop.name.try_into().unwrap();
        let now = time::macros::datetime!(2026-10-17 20:30 UTC).unix_timestamp();
        let mut results = std::vec::Vec::new();
        for source in [
            include_bytes!("fixtures/lookup-grouped.zip").as_slice(),
            include_bytes!("fixtures/lookup-shuffled.zip").as_slice(),
        ] {
            let mut builder = timetable_display::feed::Builder::new("20261017".parse().unwrap());
            let mut workspace = gtfs::zip::Workspace::new();
            builder.begin(now).unwrap();
            builder
                .calendars(source, 7, &c, &mut workspace)
                .await
                .unwrap();
            builder.trips(source, &c, &mut workspace).await.unwrap();
            builder
                .departures(source, &c, &mut workspace)
                .await
                .unwrap();
            builder.events.sort();
            results.push(builder.events.iter().collect::<std::vec::Vec<_>>());
        }
        assert_eq!(results[0], results[1]);
        let today = clock::service_epoch("20261017".parse().unwrap()).unwrap();
        let tomorrow = clock::service_epoch("20261018".parse().unwrap()).unwrap();
        assert_eq!(
            results[0]
                .iter()
                .map(|event| event.at)
                .collect::<std::vec::Vec<_>>(),
            [0, 1, 4, 7, 8]
                .map(|minute| today + 23 * 3600 + minute * 60)
                .into_iter()
                .chain([tomorrow + 30 * 60])
                .collect::<std::vec::Vec<_>>()
        );
    });
}

struct PausedFooter<'a>(&'a [u8]);
impl embedded_io_async::ErrorType for PausedFooter<'_> {
    type Error = core::convert::Infallible;
}
impl embedded_io_async::Read for PausedFooter<'_> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        if self.0.len() <= 10 {
            return core::future::pending().await;
        }
        out[0] = self.0[0];
        self.0 = &self.0[1..];
        Ok(1)
    }
}

#[test]
fn settings_crc_covers_generation_and_recovers_only_complete_images() {
    let c = settings();
    let mut browser_image = *include_bytes!("settings.bin");
    assert!(config::decode(&mut browser_image).unwrap().0 == c);
    let mut image = [0; config::IMAGE_SIZE];
    config::encode(&c, u32::MAX, &mut image).unwrap();
    let (copy, generation) = config::decode(&mut image).unwrap();
    assert!(copy == c);
    assert_eq!(generation, u32::MAX);
    for at in [8, 12, 16, 20, 30, 100] {
        let mut bad = image;
        bad[at] ^= 1;
        assert!(config::decode(&mut bad).is_err());
    }
    assert!(config::newer(0, u32::MAX));
    assert!(!config::newer(u32::MAX, 0));
    assert!(!config::newer(2, 2));
}

#[test]
fn browser_and_unicode_escaped_credentials_survive_flash_roundtrips() {
    let mut image = *include_bytes!("settings-escaped.bin");
    let (expected, _) = config::decode(&mut image).unwrap();
    assert_eq!(expected.wifi_ssid, "Home \"2.4\" \\ Łódź");
    assert_eq!(expected.wifi_password, "pa\\ss\"word");
    assert_eq!(expected.groups[0].name, "Work \"home\"");
    assert_eq!(
        expected.groups[0].picks[0].stop_name,
        "\"Łąkowa\" \\ północ"
    );
    assert!(expected.battery);
    config::encode(&expected, 17, &mut image).unwrap();
    assert!(config::decode(&mut image).unwrap().0 == expected);
    let mut limits = expected.clone();
    limits.wifi_ssid = "\\".repeat(32).as_str().try_into().unwrap();
    limits.wifi_password = "\\".repeat(63).as_str().try_into().unwrap();
    limits.groups[0].picks[0].stop_name = "\"".repeat(64).as_str().try_into().unwrap();
    config::encode(&limits, 18, &mut image).unwrap();
    assert!(config::decode(&mut image).unwrap().0 == limits);
    config::encode(&expected, 17, &mut image).unwrap();
    // JSON producers may encode Unicode rather than writing UTF-8 literally.
    let payload = include_bytes!("settings-escaped.json");
    image[config::HEADER_SIZE..].fill(0xff);
    image[config::HEADER_SIZE..config::HEADER_SIZE + payload.len()].copy_from_slice(payload);
    image[12..16].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    image[16..20].fill(0);
    let crc = config::crc32(&image[..config::HEADER_SIZE + payload.len()]);
    image[16..20].copy_from_slice(&crc.to_le_bytes());
    assert!(config::decode(&mut image).unwrap().0 == expected);
}

#[test]
fn legacy_full_settings_reencode_without_growing_a_default_field() {
    let mut c = settings();
    let mut p = c.groups[0].picks[0].clone();
    p.route = "\"".repeat(32).as_str().try_into().unwrap();
    p.line = "\"".repeat(8).as_str().try_into().unwrap();
    p.stop = "\"".repeat(32).as_str().try_into().unwrap();
    p.stop_name = "\"".repeat(64).as_str().try_into().unwrap();
    p.stop_code = "\"".repeat(16).as_str().try_into().unwrap();
    p.headsign = "\"".repeat(64).as_str().try_into().unwrap();
    let mut g = c.groups[0].clone();
    g.name = "\"".repeat(24).as_str().try_into().unwrap();
    g.picks.clear();
    for _ in 0..4 {
        g.picks.push(p.clone()).unwrap();
    }
    c.groups.clear();
    for _ in 0..4 {
        c.groups.push(g.clone()).unwrap();
    }
    let mut json = [0; 16384];
    let limit = config::IMAGE_SIZE - config::HEADER_SIZE;
    loop {
        let len = serde_json_core::to_slice(&c, &mut json).unwrap();
        if len <= limit {
            assert!(len >= limit - 1);
            assert!(
                !core::str::from_utf8(&json[..len])
                    .unwrap()
                    .contains("battery")
            );
            break;
        }
        c.groups
            .iter_mut()
            .flat_map(|g| &mut g.picks)
            .find(|p| !p.headsign.is_empty())
            .unwrap()
            .headsign
            .pop();
    }
    let mut image = [0; config::IMAGE_SIZE];
    config::encode(&c, 99, &mut image).unwrap();
    let (decoded, generation) = config::decode(&mut image).unwrap();
    assert!(!decoded.battery);
    config::encode(&decoded, generation, &mut image).unwrap();
    assert!(config::decode(&mut image).unwrap().0 == c);
    c.battery = true;
    assert_eq!(
        config::encode(&c, 100, &mut image),
        Err(config::Error::Capacity)
    );
}

#[test]
fn bounded_settings_parser_keeps_strict_schema_and_full_capacity() {
    fn parse(payload: &str) -> Result<Config, config::Error> {
        let mut image = [0xff; config::IMAGE_SIZE];
        image[..8].copy_from_slice(config::MAGIC);
        image[8..12].fill(0);
        image[12..16].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        image[16..24].fill(0);
        image[24..24 + payload.len()].copy_from_slice(payload.as_bytes());
        let crc = config::crc32(&image[..24 + payload.len()]);
        image[16..20].copy_from_slice(&crc.to_le_bytes());
        config::decode(&mut image).map(|(c, _)| c)
    }
    let pick = r#"{"route":"4","line":"4","stop":"S1","stop_name":"Stop","stop_code":"S01","headsign":"Town","direction":0,"walk":6,"spread":1,"margin":1}"#;
    let group = format!(r#"{{"name":"Work","days":31,"start":360,"end":660,"picks":[{pick}]}}"#);
    let json =
        format!(r#"{{"wifi_ssid":"test","wifi_password":"","automatic":true,"groups":[{group}]}}"#);
    let c = parse(&json).unwrap();
    assert!(!c.battery);
    let reordered = format!(
        r#"{{"groups":[{group}],"battery":false,"automatic":true,"wifi_password":"","wifi_\u0073sid":"test"}}"#
    );
    assert!(parse(&reordered).unwrap() == c);
    for malformed in [
        json.replace(r#""wifi_ssid":"test","#, ""),
        json.replace(r#""wifi_password":"","#, ""),
        json.replace(r#""automatic":true,"#, ""),
        json.replace(
            r#""wifi_ssid":"test""#,
            r#""wifi_ssid":"test","wifi_ssid":"other""#,
        ),
        json.replace(
            r#""groups":["#,
            r#""battery":true,"battery":false,"groups":["#,
        ),
        json.replace(r#""groups":["#, r#""extra":0,"groups":["#),
        json.replace(r#""name":"Work","#, ""),
        json.replace(r#""days":31,"#, ""),
        json.replace(r#""start":360,"#, ""),
        json.replace(r#""end":660,"#, ""),
        json.replace(r#""picks":["#, r#""days":31,"picks":["#),
        json.replace(r#""picks":["#, r#""extra":0,"picks":["#),
        json.replace(r#""margin":1"#, r#""margin":1,"extra":0"#),
        json.replace(r#""walk":6"#, r#""walk":6,"walk":7"#),
        json.replace(r#""days":31"#, r#""days":null"#),
        json.replace(r#""automatic":true"#, r#""automatic":1"#),
        format!("{json} false"),
    ] {
        assert!(parse(&malformed).is_err(), "accepted {malformed}");
    }
    let full_group = group.replace(pick, &[pick; 4].join(","));
    let full = json.replace(&group, &[full_group.as_str(); 4].join(","));
    let full = parse(&full).unwrap();
    assert_eq!(full.groups.len(), 4);
    assert!(full.groups.iter().all(|g| g.picks.len() == 4));
    let mut snapshot = Config::empty();
    snapshot.clone_from(&full);
    assert!(snapshot == full);
    snapshot.battery = true;
    snapshot.clone_from(&c);
    assert!(snapshot == c);
    assert!(parse(&json.replace(&group, &[group.as_str(); 5].join(","))).is_err());
    assert!(parse(&json.replace(pick, &[pick; 5].join(","))).is_err());
}

#[test]
fn overnight_groups_use_the_day_the_window_started() {
    let mut g = settings().groups.remove(0);
    g.start = 22 * 60;
    g.end = 2 * 60;
    g.days = 1 << 4;
    assert!(g.active(4, 23 * 60));
    assert!(g.active(5, 60));
    assert!(!g.active(4, 60));
    assert!(!g.active(5, 120));
}

#[test]
fn walking_estimates_have_explicit_boundaries_and_a_boarding_buffer() {
    let c = settings();
    let p = &c.groups[0].picks[0];
    // Uniform walking interval [4,8] min, plus 1 minute boarding buffer.
    assert_eq!(model::walking_chance(0, 0, p), 0);
    assert_eq!(model::walking_chance(5 * 60, 0, p), 0);
    assert_eq!(model::walking_chance(7 * 60, 0, p), 50);
    assert_eq!(model::walking_chance(9 * 60, 0, p), 100);
    assert_eq!(model::leave_in(10 * 60, 0, p), 3 * 60);
    let mut p = p.clone();
    p.spread = 0;
    assert_eq!(model::walking_chance(7 * 60, 0, &p), 100);
    assert_eq!(model::walking_chance(7 * 60 - 1, 0, &p), 0);
}

#[test]
fn warsaw_dst_and_gtfs_service_day_survive_midnight() {
    use time::macros::datetime;
    let march = datetime!(2026-03-29 01:00 UTC).unix_timestamp();
    assert_eq!(clock::warsaw(march - 1).unwrap().hour(), 1);
    assert_eq!(clock::warsaw(march).unwrap().hour(), 3);
    let october = datetime!(2026-10-25 01:00 UTC).unix_timestamp();
    assert_eq!(clock::warsaw(october - 1).unwrap().hour(), 2);
    assert_eq!(clock::warsaw(october).unwrap().hour(), 2);
    assert_eq!(
        clock::warsaw(october - 1).unwrap().offset().whole_hours(),
        2
    );
    assert_eq!(clock::warsaw(october).unwrap().offset().whole_hours(), 1);
    let date = "20261017".parse().unwrap();
    let late = clock::service_epoch(date).unwrap() + 25 * 3600 + 10 * 60;
    assert_eq!(clock::service_date(late).unwrap().to_string(), "20261018");
    assert_eq!(clock::warsaw(late).unwrap().hour(), 1);
    assert_eq!(
        clock::previous("20260301".parse().unwrap())
            .unwrap()
            .to_string(),
        "20260228"
    );
}

#[test]
fn catalogue_is_read_from_flash_and_keeps_directional_stop_membership() {
    let bytes = include_bytes!("../../catalog/catalog.bin");
    let c = Catalog::new(bytes).unwrap();
    assert!(c.route_count() > 100 && c.stop_count() > 3000);
    let route = (0..c.route_count())
        .find(|&i| c.route(i).unwrap().id == "4")
        .unwrap();
    assert!(c.route_stops(route, -1).count() > 20);
    assert!(
        c.route_stops(route, 0)
            .all(|(i, _)| c.route_stops(route, -1).any(|(j, _)| i == j))
    );
    let mut corrupt = bytes.to_vec();
    corrupt[100] ^= 1;
    assert!(Catalog::new(&corrupt).is_none());
}

#[test]
fn catalogue_lookup_preserves_unsorted_link_support() {
    let original = include_bytes!("fixtures/catalog.bin");
    let sorted = Catalog::new(original).unwrap();
    let mut bytes = original.to_vec();
    let links = 32 + sorted.route_count() * 168 + sorted.stop_count() * 112;
    for i in 0..6 {
        bytes.swap(links + i, links + 6 + i);
    }
    let crc = config::crc32(&bytes[32..]);
    bytes[24..28].copy_from_slice(&crc.to_le_bytes());
    let unsorted = Catalog::new(&bytes).unwrap();
    for route in 0..=sorted.route_count() {
        for direction in [-1, 0, 1] {
            let ids = |catalog: &Catalog<'_>| {
                catalog
                    .route_stops(route, direction)
                    .map(|(i, _)| i)
                    .collect::<std::vec::Vec<_>>()
            };
            assert_eq!(ids(&sorted), ids(&unsorted));
        }
    }
}

#[test]
fn device_editing_keeps_the_current_choice_and_back_steps_without_saving() {
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let route = (0..catalog.route_count())
        .find(|&i| catalog.route(i).unwrap().line == "4")
        .unwrap();
    let (_, stop) = catalog.route_stops(route, 0).nth(9).unwrap();
    let mut c = settings();
    let pick = &mut c.groups[0].picks[0];
    pick.stop = stop.id.try_into().unwrap();
    pick.stop_name = stop.name.try_into().unwrap();
    pick.stop_code = stop.code.try_into().unwrap();
    let original = c.clone();
    let mut app = App::default();
    ui::tap(&mut app, &mut c, &catalog, 120, 120, false);
    assert_eq!(app.screen, Screen::Board);
    ui::tap(&mut app, &mut c, &catalog, 120, 120, true);
    ui::tap(&mut app, &mut c, &catalog, 120, 65, false);
    assert_eq!(app.screen, Screen::Walk);
    app.draft.as_mut().unwrap().walk = 8;
    ui::tap(&mut app, &mut c, &catalog, 150, 48, false);
    assert_eq!(app.screen, Screen::Stops);
    assert_eq!(
        (app.selected_route, app.direction, app.cursor),
        (route, 0, 8)
    );
    ui::tap(&mut app, &mut c, &catalog, 45, 25, false);
    assert_eq!(app.screen, Screen::Routes);
    assert_eq!(app.cursor, route / 4 * 4);
    ui::tap(&mut app, &mut c, &catalog, 45, 25, false);
    assert_eq!(app.screen, Screen::Walk);
    assert_eq!(app.draft.as_ref().unwrap().walk, 8);
    ui::tap(&mut app, &mut c, &catalog, 45, 25, false);
    assert_eq!(app.screen, Screen::Picks);
    assert!(app.draft.is_none());
    assert!(c == original);
    // Cancelling a new choice requires no flash write.
    ui::tap(&mut app, &mut c, &catalog, 120, 90, false);
    assert_eq!(app.screen, Screen::Routes);
    ui::tap(&mut app, &mut c, &catalog, 120, 65, false);
    assert_eq!(app.screen, Screen::Stops);
    ui::tap(&mut app, &mut c, &catalog, 120, 65, false);
    assert_eq!(app.screen, Screen::Walk);
    assert_eq!(
        ui::tap(&mut app, &mut c, &catalog, 70, 201, false),
        ui::Action::None
    );
    assert_eq!(app.screen, Screen::Picks);
    assert!(c == original);
}

#[test]
fn device_preselection_uses_the_saved_platform_when_an_id_is_reused() {
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let route = (0..catalog.route_count())
        .find(|&i| catalog.route(i).unwrap().line == "4")
        .unwrap();
    let (_, desired) = catalog.route_stops(route, 0).next().unwrap();
    let (_, reused) = catalog.route_stops(route, 0).nth(9).unwrap();
    assert_ne!(desired.code, reused.code);
    let mut c = settings();
    let pick = &mut c.groups[0].picks[0];
    pick.stop = reused.id.try_into().unwrap();
    pick.stop_code = desired.code.try_into().unwrap();
    pick.stop_name = desired.name.try_into().unwrap();
    c.validate().unwrap();
    let mut app = App::default();
    ui::tap(&mut app, &mut c, &catalog, 120, 120, true);
    ui::tap(&mut app, &mut c, &catalog, 120, 65, false);
    ui::tap(&mut app, &mut c, &catalog, 150, 48, false);
    assert_eq!(app.screen, Screen::Stops);
    assert_eq!(app.cursor, 0);
    ui::tap(&mut app, &mut c, &catalog, 120, 65, false);
    assert_eq!(app.draft.as_ref().unwrap().stop, desired.id);
    assert_eq!(app.draft.as_ref().unwrap().stop_code, desired.code);
    assert_eq!(
        ui::tap(&mut app, &mut c, &catalog, 150, 201, false),
        ui::Action::Save { refresh: true }
    );
    assert_eq!(c.groups[0].picks[0].stop, desired.id);
}

#[test]
fn editing_is_provisional_and_manual_groups_expire_after_two_hours() {
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let mut c = settings();
    let original = c.clone();
    let mut app = App {
        clock: Some(100),
        ..App::default()
    };
    ui::tap(&mut app, &mut c, &catalog, 120, 120, true);
    assert_eq!(app.screen, Screen::Picks);
    ui::tap(&mut app, &mut c, &catalog, 120, 65, false);
    ui::tap(&mut app, &mut c, &catalog, 180, 78, false);
    assert_eq!(app.draft.as_ref().unwrap().walk, 7);
    ui::tap(&mut app, &mut c, &catalog, 45, 25, false);
    assert!(c == original);
    app.choose_group(1, &c);
    assert_eq!(app.manual_until, 7300);
    app.tick(&c, 200);
    assert_eq!(app.group, 1);
    let mut departures = timetable_display::departures::Departures::new(200);
    departures
        .push(Departure {
            at: 1000,
            group: 0,
            pick: 0,
            conditional: false,
            headsign: 0,
        })
        .unwrap();
    app.publish(departures, model::Headings::default(), 200);
    assert!(app.upcoming().next().is_none());
}

#[test]
fn streaming_device_query_uses_exceptions_and_rolls_back_a_late_footer_error() {
    futures::executor::block_on(async {
        let mut c = settings();
        c.groups[0].picks[0].route = "obsolete-route-id".try_into().unwrap();
        c.groups[0].picks[0].stop = "obsolete-stop-id".try_into().unwrap();
        let now = time::macros::datetime!(2026-10-17 05:30 UTC).unix_timestamp();
        let mut builder = timetable_display::feed::Builder::new("20261017".parse().unwrap());
        let mut workspace = gtfs::zip::Workspace::new();
        let source = include_bytes!("../../gtfs/tests/fixtures/gtfs.zip");
        builder.begin(now).unwrap();
        builder
            .calendars(source.as_slice(), 3, &c, &mut workspace)
            .await
            .unwrap();
        builder
            .trips(
                include_bytes!("../../gtfs/tests/fixtures/shuffled.zip").as_slice(),
                &c,
                &mut workspace,
            )
            .await
            .unwrap();
        builder
            .departures(source.as_slice(), &c, &mut workspace)
            .await
            .unwrap();
        assert_eq!(builder.events.len(), 1);
        assert_eq!(
            builder
                .headsigns
                .get(builder.events.get(0).unwrap().headsign),
            Some("Branch")
        );
        assert_eq!(
            clock::warsaw(builder.events.get(0).unwrap().at)
                .unwrap()
                .hour(),
            9
        );
        builder.events.clear();
        assert!(
            builder
                .departures(&source[..source.len() - 1], &c, &mut workspace)
                .await
                .is_err()
        );
        assert!(builder.events.is_empty());
        use futures::FutureExt;
        assert!(
            builder
                .departures(PausedFooter(source), &c, &mut workspace)
                .now_or_never()
                .is_none()
        );
        assert!(builder.events.is_empty());
    });
}

#[test]
fn previous_service_day_supplies_a_25_hour_departure_after_midnight() {
    futures::executor::block_on(async {
        let mut c = settings();
        c.groups[0].picks[0].stop = "S1".try_into().unwrap();
        let now = time::macros::datetime!(2026-10-17 22:30 UTC).unix_timestamp();
        let mut builder = timetable_display::feed::Builder::new("20261018".parse().unwrap());
        let mut workspace = gtfs::zip::Workspace::new();
        let source = include_bytes!("../../gtfs/tests/fixtures/gtfs.zip").as_slice();
        builder.begin(now).unwrap();
        builder
            .calendars(source, 3, &c, &mut workspace)
            .await
            .unwrap();
        builder.trips(source, &c, &mut workspace).await.unwrap();
        builder
            .departures(source, &c, &mut workspace)
            .await
            .unwrap();
        assert_eq!(builder.events.len(), 1);
        let date = clock::warsaw(builder.events.get(0).unwrap().at).unwrap();
        assert_eq!((date.day(), date.hour(), date.minute()), (18, 1, 10));
    });
}

#[test]
fn spring_dst_service_day_is_included_before_civil_midnight() {
    futures::executor::block_on(async {
        use time::macros::datetime;
        let catalog = Catalog::new(include_bytes!("fixtures/catalog.bin")).unwrap();
        let stop = catalog.stop(0).unwrap();
        let mut c = settings();
        let pick = &mut c.groups[0].picks[0];
        pick.route = "R4".try_into().unwrap();
        pick.stop = stop.id.try_into().unwrap();
        pick.stop_code.clear();
        pick.stop_name = stop.name.try_into().unwrap();
        let mut builder = timetable_display::feed::Builder::new("20260328".parse().unwrap());
        let mut workspace = gtfs::zip::Workspace::new();
        let source = include_bytes!("fixtures/spring-dst.zip").as_slice();
        let now = datetime!(2026-03-28 16:45 UTC).unix_timestamp();
        builder.begin(now).unwrap();
        assert_eq!(clock::warsaw(now + 6 * 3600).unwrap().day(), 28);
        builder
            .calendars(source, builder.day_mask(), &c, &mut workspace)
            .await
            .unwrap();
        builder.trips(source, &c, &mut workspace).await.unwrap();
        builder
            .departures(source, &c, &mut workspace)
            .await
            .unwrap();
        builder.events.sort();
        // Sunday's 00:30 GTFS time is Saturday 23:30 civil time on this transition.
        assert_eq!(builder.events.len(), 2);
        assert_eq!(
            builder.events.get(0).unwrap().at,
            datetime!(2026-03-28 22:30 UTC).unix_timestamp()
        );
        assert_eq!(
            builder.events.get(1).unwrap().at,
            datetime!(2026-03-28 22:45 UTC).unix_timestamp()
        );
        let tomorrow = clock::service_epoch("20260329".parse().unwrap()).unwrap();
        builder.begin(tomorrow - 6 * 3600 - 1).unwrap();
        assert_eq!(builder.day_mask(), 3);
        builder.begin(tomorrow - 6 * 3600).unwrap();
        assert_eq!(builder.day_mask(), 7);
    });
}

#[test]
fn evening_queries_include_next_day_service_and_rollback_headsigns() {
    futures::executor::block_on(async {
        let catalog = Catalog::new(include_bytes!("fixtures/catalog.bin")).unwrap();
        // The drop-off-only terminus is not a selectable boarding stop.
        assert_eq!(catalog.stop_count(), 2);
        assert!((0..catalog.stop_count()).any(|i| catalog.stop(i).unwrap().id == "S3"));
        assert!((0..catalog.stop_count()).all(|i| catalog.stop(i).unwrap().id != "S2"));
        assert_eq!(catalog.route_stops(1, -1).count(), 1);
        assert_eq!(catalog.route_stops(1, 0).count(), 0);
        assert_eq!(catalog.route_stops(1, 1).count(), 0);
        let stop = catalog.stop(0).unwrap();
        let mut c = settings();
        let p = &mut c.groups[0].picks[0];
        p.route = "R4".try_into().unwrap();
        p.stop = stop.id.try_into().unwrap();
        p.stop_code.clear();
        p.stop_name = stop.name.try_into().unwrap();
        let now = time::macros::datetime!(2026-10-17 20:30 UTC).unix_timestamp();
        let source = include_bytes!("fixtures/edge.zip").as_slice();
        let mut builder = timetable_display::feed::Builder::new("20261017".parse().unwrap());
        let mut workspace = gtfs::zip::Workspace::new();
        builder.begin(now).unwrap();
        assert_eq!(builder.day_mask(), 7);
        builder
            .calendars(source, 7, &c, &mut workspace)
            .await
            .unwrap();
        assert!(
            builder
                .trips(&source[..source.len() - 1], &c, &mut workspace)
                .await
                .is_err()
        );
        assert_eq!(builder.headsigns.get(0), None);
        use futures::FutureExt;
        assert!(
            builder
                .trips(PausedFooter(source), &c, &mut workspace)
                .now_or_never()
                .is_none()
        );
        assert_eq!(builder.headsigns.get(0), None);
        builder.trips(source, &c, &mut workspace).await.unwrap();
        assert_eq!(builder.headsigns.get(0), Some("Trip branch"));
        assert_eq!(builder.headsigns.get(1), None);
        assert!(
            builder
                .departures(&source[..source.len() - 1], &c, &mut workspace)
                .await
                .is_err()
        );
        assert!(builder.events.is_empty());
        assert_eq!(builder.headsigns.get(1), None);
        assert!(
            builder
                .departures(PausedFooter(source), &c, &mut workspace)
                .now_or_never()
                .is_none()
        );
        assert_eq!(builder.headsigns.get(1), None);
        builder
            .departures(source, &c, &mut workspace)
            .await
            .unwrap();
        builder.events.sort();
        assert_eq!(builder.events.len(), 2);
        let tomorrow = clock::warsaw(builder.events.get(1).unwrap().at).unwrap();
        assert_eq!(
            (tomorrow.day(), tomorrow.hour(), tomorrow.minute()),
            (18, 0, 30)
        );
        assert_eq!(
            builder
                .headsigns
                .get(builder.events.get(1).unwrap().headsign),
            Some("Platform branch")
        );
        let mut app = App::default();
        app.headsigns.insert("Old snapshot destination").unwrap();
        app.publish(
            core::mem::take(&mut builder.events),
            core::mem::take(&mut builder.headsigns),
            now,
        );
        assert_eq!(
            app.headsigns.get(app.departures.get(1).unwrap().headsign),
            Some("Platform branch")
        );
        // When the saved exact route disappears, duplicate public names fail.
        c.groups[0].picks[0].route = "old-route".try_into().unwrap();
        assert!(matches!(
            builder.calendars(source, 7, &c, &mut workspace).await,
            Err(gtfs::Error::InvalidField("ambiguous line"))
        ));
        let p = &mut c.groups[0].picks[0];
        p.route = "R5".try_into().unwrap();
        p.stop = "S3".try_into().unwrap();
        p.stop_code = "T02".try_into().unwrap();
        p.direction = -1;
        builder.begin(now).unwrap();
        builder
            .calendars(source, 7, &c, &mut workspace)
            .await
            .unwrap();
        builder.trips(source, &c, &mut workspace).await.unwrap();
        builder
            .departures(source, &c, &mut workspace)
            .await
            .unwrap();
        assert_eq!(builder.events.len(), 1);
        c.groups[0].picks[0].stop = "stale-stop".try_into().unwrap();
        assert!(matches!(
            builder.calendars(source, 7, &c, &mut workspace).await,
            Err(gtfs::Error::InvalidField("ambiguous stop code"))
        ));
    });
}
