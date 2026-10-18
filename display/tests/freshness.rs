use time::macros::datetime;
use timetable_display::{
    App, Config, Screen,
    catalog::Catalog,
    freshness::{Freshness, SourceTimes, http_date, source_modified},
    model::Status,
    render, ui,
};

#[test]
fn age_details_and_retry_keep_the_validated_metadata_and_settings() {
    let mut config: Config = serde_json_core::from_slice(include_bytes!("settings.json"))
        .unwrap()
        .0;
    let original = config.clone();
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let mut app = App {
        status: Status::Failed,
        loaded_at: Some(100),
        freshness: Freshness {
            checked_at: Some(110),
            source_updated_at: Some(90),
        },
        ..App::default()
    };
    let freshness = app.freshness;
    assert_eq!(
        ui::tap(&mut app, &mut config, &catalog, 120, 212, false),
        ui::Action::None
    );
    assert_eq!(app.screen, Screen::Freshness);
    assert_eq!(
        ui::tap(&mut app, &mut config, &catalog, 120, 207, false),
        ui::Action::Refresh
    );
    assert_eq!(app.screen, Screen::Board);
    assert_eq!(app.status, Status::Loading);
    assert_eq!(app.freshness, freshness);
    assert_eq!(app.loaded_at, Some(100));
    assert!(config == original);
    ui::tap(&mut app, &mut config, &catalog, 120, 212, false);
    assert_eq!(app.screen, Screen::Freshness);
    ui::tap(&mut app, &mut config, &catalog, 45, 25, false);
    assert_eq!(app.screen, Screen::Board);
}

#[test]
fn http_dates_accept_all_recipient_formats_and_post_2038_time() {
    let now = datetime!(2026-10-18 12:00 UTC).unix_timestamp();
    let expected = datetime!(1994-11-06 08:49:37 UTC).unix_timestamp();
    for value in [
        "Sun, 06 Nov 1994 08:49:37 GMT",
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
    ] {
        assert_eq!(http_date(value.as_bytes(), now), Some(expected));
    }
    assert_eq!(
        http_date(b"Tue, 19 Jan 2038 03:14:09 GMT", now),
        Some(2_147_483_649)
    );
    assert_eq!(
        http_date(b"Sat, 31 Dec 2016 23:59:60 GMT", now),
        Some(datetime!(2017-01-01 00:00 UTC).unix_timestamp())
    );
}

#[test]
fn malformed_source_dates_do_not_become_freshness_claims() {
    for value in [
        "",
        "Sun, 06 Nov 1994 08:49:37 GMT extra",
        "Mon, 06 Nov 1994 08:49:37 GMT",
        "Sun, 31 Feb 2026 08:49:37 GMT",
        "Sun, 06 Nov 1994 24:00:00 GMT",
        "Sun, 06 Nov 1994 08:49:61 GMT",
        "Sun, 06 Nov 1994 08:49:37 CET",
        "Sunday, 06/Nov/94 08:49:37 GMT",
        "éé, 06 Nov 1994 08:49:37 GMT",
        "Sun, +6 Nov 1994 08:49:37 GMT",
    ] {
        assert_eq!(http_date(value.as_bytes(), 1_792_324_800), None, "{value}");
    }
}

#[test]
fn obsolete_two_digit_year_uses_the_full_fifty_year_boundary() {
    let now = datetime!(2026-10-18 12:00 UTC).unix_timestamp();
    assert_eq!(
        http_date(b"Friday, 31-Dec-76 08:49:37 GMT", now),
        Some(datetime!(1976-12-31 08:49:37 UTC).unix_timestamp())
    );
    assert_eq!(
        http_date(b"Sunday, 18-Oct-76 12:00:00 GMT", now),
        Some(datetime!(2076-10-18 12:00 UTC).unix_timestamp())
    );
    assert_eq!(
        http_date(b"Monday, 18-Oct-76 12:00:01 GMT", now),
        Some(datetime!(1976-10-18 12:00:01 UTC).unix_timestamp())
    );
}

#[test]
fn mixed_archives_use_the_oldest_timestamp_and_missing_metadata_stays_unknown() {
    let mut times = SourceTimes::default();
    assert_eq!(times.updated_at(), None);
    for stamp in [Some(300), Some(300), Some(200), Some(250)] {
        times.observe(stamp);
    }
    assert_eq!(times.updated_at(), Some(200));
    times.observe(None);
    times.observe(Some(400));
    assert_eq!(times.updated_at(), None);
}

#[test]
fn last_modified_is_distinct_from_http_date_and_must_be_unambiguous() {
    let now = datetime!(2026-10-18 12:00 UTC).unix_timestamp();
    let modified = b"Thu, 15 Oct 2026 04:00:00 GMT".as_slice();
    assert_eq!(source_modified([("Date", modified)], now), None);
    assert_eq!(
        source_modified([("lAsT-mOdIfIeD", modified)], now),
        Some(datetime!(2026-10-15 04:00 UTC).unix_timestamp())
    );
    assert_eq!(
        source_modified(
            [("Last-Modified", modified), ("last-modified", modified)],
            now
        ),
        None
    );
    assert_eq!(
        source_modified(
            [("Last-Modified", b"Mon, 19 Oct 2026 04:00:00 GMT".as_slice())],
            now
        ),
        None
    );
}

#[test]
fn compact_footer_distinguishes_today_old_checks_and_unknown_source() {
    let mut app = App {
        clock: Some(datetime!(2026-10-18 12:00 UTC).unix_timestamp()),
        freshness: Freshness {
            checked_at: Some(datetime!(2026-10-18 11:30 UTC).unix_timestamp()),
            source_updated_at: Some(datetime!(2026-10-15 04:00 UTC).unix_timestamp()),
        },
        ..App::default()
    };
    assert_eq!(render::freshness_line(&app), "Chk13:30 Src15/10");
    app.clock = Some(datetime!(2026-10-18 22:01 UTC).unix_timestamp());
    assert_eq!(render::freshness_line(&app), "Chk18/10 Src15/10");
    app.freshness.source_updated_at = None;
    assert_eq!(render::freshness_line(&app), "Chk18/10 Src --/--");
    app.freshness = Freshness::default();
    assert_eq!(render::freshness_line(&app), "Chk --:-- Src --/--");
    assert!(render::freshness_line(&app).len() <= 23);
}
