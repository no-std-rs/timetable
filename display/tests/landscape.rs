use timetable_display::{App, Config, Screen, catalog::Catalog, ui};
fn settings() -> Config {
    serde_json_core::from_slice::<Config>(include_bytes!("settings.json"))
        .unwrap()
        .0
}
fn catalog() -> Catalog<'static> {
    Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap()
}
#[test]
fn route_paging_visits_every_choice_and_both_large_rows_select_the_displayed_route() {
    let cat = catalog();
    let mut config = settings();
    let mut app = App {
        screen: Screen::Routes,
        ..App::default()
    };
    let mut seen = Vec::new();
    loop {
        for i in app.cursor..(app.cursor + 2).min(cat.route_count()) {
            seen.push(i);
        }
        let previous = app.cursor;
        ui::tap_landscape(&mut app, &mut config, &cat, 286, 150, false);
        if app.cursor == previous {
            break;
        }
        assert_eq!(app.cursor, previous + 2);
    }
    assert_eq!(seen, (0..cat.route_count()).collect::<Vec<_>>());
    for selected in 0..cat.route_count() {
        app.screen = Screen::Routes;
        app.cursor = selected / 2 * 2;
        ui::tap_landscape(
            &mut app,
            &mut config,
            &cat,
            160,
            56 + (selected % 2) as u16 * 48,
            false,
        );
        assert_eq!(app.screen, Screen::Stops);
        assert_eq!(app.selected_route, selected);
        ui::tap_landscape(&mut app, &mut config, &cat, 12, 8, false);
        assert_eq!(app.screen, Screen::Routes);
        assert_eq!(app.cursor, selected / 2 * 2);
    }
    app.cursor = 2;
    ui::swipe_landscape(&mut app, &config, &cat, 0, -80);
    assert_eq!(app.cursor, 4);
    ui::swipe_landscape(&mut app, &config, &cat, 0, 80);
    assert_eq!(app.cursor, 2);
}
#[test]
fn all_four_choice_cards_are_reachable_and_walk_controls_do_not_cross_fields() {
    let cat = catalog();
    let mut config = settings();
    let original = config.groups[0].picks[0].clone();
    config.groups[0].picks.clear();
    for walk in 10..14 {
        let mut pick = original.clone();
        pick.walk = walk;
        config.groups[0].picks.push(pick).unwrap();
    }
    for (i, (x, y)) in [(80, 56), (240, 56), (80, 104), (240, 104)]
        .into_iter()
        .enumerate()
    {
        let mut app = App {
            screen: Screen::Picks,
            ..App::default()
        };
        ui::tap_landscape(&mut app, &mut config, &cat, x, y, false);
        assert_eq!(app.screen, Screen::Walk);
        assert_eq!(app.draft.as_ref().unwrap().walk, 10 + i as u8);
        let before = app.draft.clone().unwrap();
        ui::tap_landscape(&mut app, &mut config, &cat, 80, 100, false);
        ui::tap_landscape(&mut app, &mut config, &cat, 186, 100, false);
        ui::tap_landscape(&mut app, &mut config, &cat, 292, 100, false);
        let after = app.draft.as_ref().unwrap();
        assert_eq!(after.walk, before.walk + 1);
        assert_eq!(after.spread, before.spread + 1);
        assert_eq!(after.margin, before.margin + 1);
        // The empty gap between Remove and Save has no destructive action.
        ui::tap_landscape(&mut app, &mut config, &cat, 160, 150, false);
        assert_eq!(app.screen, Screen::Walk);
        assert_eq!(config.groups[0].picks[i], before);
        assert_eq!(
            ui::tap_landscape(&mut app, &mut config, &cat, 260, 150, false),
            ui::Action::Save { refresh: false }
        );
        assert_eq!(app.screen, Screen::Board);
        assert_eq!(config.groups[0].picks[i].walk, before.walk + 1);
    }
}
#[test]
fn editing_a_saved_stop_opens_the_page_that_contains_it() {
    let cat = catalog();
    let mut config = settings();
    let mut pick = config.groups[0].picks[0].clone();
    let route = (0..cat.route_count())
        .find(|&i| cat.route(i).unwrap().id == pick.route)
        .unwrap();
    let (_, stop) = cat.route_stops(route, pick.direction).nth(3).unwrap();
    pick.stop = stop.id.try_into().unwrap();
    pick.stop_name = stop.name.try_into().unwrap();
    pick.stop_code = stop.code.try_into().unwrap();
    config.groups[0].picks[0] = pick.clone();
    let position = 3;
    let mut app = App {
        screen: Screen::Walk,
        draft: Some(pick),
        ..App::default()
    };
    ui::tap_landscape(&mut app, &mut config, &cat, 200, 44, false);
    assert_eq!(app.screen, Screen::Stops);
    assert_eq!(app.cursor, position / 2 * 2);
    ui::tap_landscape(
        &mut app,
        &mut config,
        &cat,
        160,
        56 + (position % 2) as u16 * 48,
        false,
    );
    assert_eq!(app.screen, Screen::Walk);
    assert_eq!(
        app.draft.as_ref().unwrap().stop,
        config.groups[0].picks[0].stop
    );
}
