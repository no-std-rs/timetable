use gtfs::Error;
use timetable_display::{
    Departure,
    departures::{Departures, HORIZON_SECONDS},
    model::MAX_DEPARTURES,
};

#[test]
fn compact_cache_matches_full_timestamps_at_capacity_and_across_2038() {
    for epoch in [
        -5_000_000_000,
        1_792_260_000,
        4_102_444_800,
        i64::MAX - HORIZON_SECONDS,
    ] {
        let mut cache = Departures::new(epoch);
        let mut expected = heapless::Vec::<Departure, MAX_DEPARTURES>::new();
        for i in 0..MAX_DEPARTURES {
            let departure = Departure {
                at: epoch + HORIZON_SECONDS - i as i64 * 37,
                group: (i / 32 % 4) as u8,
                pick: (i / 128 % 4) as u8,
                conditional: i % 2 != 0,
                headsign: (i % 256) as u8,
            };
            cache.push(departure).unwrap();
            expected.push(departure).unwrap();
            assert_eq!(cache.get(i), Some(departure));
        }
        assert_eq!(cache.push(expected[0]), Err(Error::IndexFull));
        expected.sort_unstable_by_key(|event| (event.at, event.group, event.pick));
        cache.sort();
        assert!(cache.iter().eq(expected.iter().copied()));
        assert_eq!(cache.get(MAX_DEPARTURES), None);
        cache.truncate(7);
        assert_eq!(cache.len(), 7);
        assert_eq!(cache.iter().next_back(), Some(expected[6]));
        cache.reset(50);
        assert!(cache.is_empty());
        cache
            .push(Departure {
                at: 51,
                ..expected[0]
            })
            .unwrap();
        assert_eq!(cache.get(0).unwrap().at, 51);
    }
}

#[test]
fn invalid_times_and_selections_leave_the_snapshot_untouched() {
    let mut cache = Departures::new(100);
    let valid = Departure {
        at: 101,
        group: 3,
        pick: 3,
        conditional: true,
        headsign: 255,
    };
    cache.push(valid).unwrap();
    for at in [i64::MIN, 99, 100, 101 + HORIZON_SECONDS, i64::MAX] {
        assert_eq!(
            cache.push(Departure { at, ..valid }),
            Err(Error::InvalidField("departure time"))
        );
    }
    for departure in [
        Departure { group: 4, ..valid },
        Departure { pick: 4, ..valid },
    ] {
        assert_eq!(
            cache.push(departure),
            Err(Error::InvalidField("departure selection"))
        );
    }
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get(0), Some(valid));
    // Even when subtraction would overflow, fail before narrowing the time.
    cache.reset(i64::MIN);
    assert!(cache.push(valid).is_err());
    assert!(cache.is_empty());
}

#[test]
fn simultaneous_departures_remain_distinct_and_order_by_selection() {
    let mut cache = Departures::new(0);
    for (group, pick, conditional) in [(3, 3, false), (0, 1, true), (0, 1, true), (0, 0, false)] {
        cache
            .push(Departure {
                at: 1,
                group,
                pick,
                conditional,
                headsign: 7,
            })
            .unwrap();
    }
    cache.sort();
    assert_eq!(cache.len(), 4);
    assert_eq!(cache.get(0).unwrap().pick, 0);
    assert_eq!(cache.get(1), cache.get(2));
    assert_eq!(cache.get(3).unwrap().group, 3);
}

#[test]
fn saved_times_are_visible_without_becoming_upcoming_until_the_clock_is_known() {
    let mut app = timetable_display::App::default();
    let mut events = Departures::new(100);
    for (at, group) in [(110, 0), (120, 1), (130, 0)] {
        events
            .push(Departure {
                at,
                group,
                pick: 0,
                conditional: false,
                headsign: 0,
            })
            .unwrap();
    }
    app.publish(events, Default::default(), 100);
    assert_eq!(app.clock, None);
    assert_eq!(
        app.upcoming().count(),
        0,
        "a saved timestamp is not current UTC"
    );
    assert_eq!(
        app.board_departures().map(|d| d.at).collect::<Vec<_>>(),
        [110, 130]
    );
    app.clock = Some(115);
    assert_eq!(
        app.board_departures().map(|d| d.at).collect::<Vec<_>>(),
        [130]
    );
    assert_eq!(app.upcoming().map(|d| d.at).collect::<Vec<_>>(), [130]);
    app.group = 1;
    assert_eq!(
        app.board_departures().map(|d| d.at).collect::<Vec<_>>(),
        [120]
    );
}
