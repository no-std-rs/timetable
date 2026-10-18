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
    app.publish(
        heapless::Vec::from_slice(&[Departure {
            at: 1000,
            group: 0,
            pick: 0,
            conditional: false,
            headsign: 0,
        }])
        .unwrap(),
        model::Headings::default(),
        200,
    );
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
            builder.headsigns.get(builder.events[0].headsign),
            Some("Branch")
        );
        assert_eq!(clock::warsaw(builder.events[0].at).unwrap().hour(), 9);
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
        let date = clock::warsaw(builder.events[0].at).unwrap();
        assert_eq!((date.day(), date.hour(), date.minute()), (18, 1, 10));
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
        builder.events.sort_unstable_by_key(|d| d.at);
        assert_eq!(builder.events.len(), 2);
        let tomorrow = clock::warsaw(builder.events[1].at).unwrap();
        assert_eq!(
            (tomorrow.day(), tomorrow.hour(), tomorrow.minute()),
            (18, 0, 30)
        );
        assert_eq!(
            builder.headsigns.get(builder.events[1].headsign),
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
            app.headsigns.get(app.departures[1].headsign),
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
