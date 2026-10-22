use timetable_display::{App, Config, Screen, catalog::Catalog, model::CacheUpdate, ui};

#[test]
fn all_layouts_require_an_explicit_button_and_preserve_the_underlying_screen() {
    type Tap = fn(&mut App, &mut Config, &Catalog<'_>, u16, u16, bool) -> ui::Action;
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let mut config: Config = serde_json_core::from_slice(include_bytes!("settings.json"))
        .unwrap()
        .0;
    for (tap, width, y) in [
        (ui::tap as Tap, 240, 174),
        (ui::tap_portrait as Tap, 172, 254),
        (ui::tap_landscape as Tap, 320, 138),
    ] {
        let mut app = App {
            screen: Screen::Walk,
            cache_update: CacheUpdate::Pending,
            ..App::default()
        };
        app.draft = Some(config.groups[0].picks[0].clone());
        let draft = app.draft.clone();
        tap(&mut app, &mut config, &catalog, width * 3 / 4, y, true);
        tap(&mut app, &mut config, &catalog, width / 2, y, false);
        tap(&mut app, &mut config, &catalog, width / 2, 20, false);
        ui::swipe(&mut app, &config, &catalog, 0, -80);
        ui::swipe_landscape(&mut app, &config, &catalog, 0, -80);
        assert_eq!(app.cache_update, CacheUpdate::Pending);
        assert_eq!(app.screen, Screen::Walk);
        assert_eq!(app.draft, draft);
        assert_eq!(
            tap(&mut app, &mut config, &catalog, width / 4, y, false),
            ui::Action::None
        );
        assert_eq!(app.cache_update, CacheUpdate::Declined);
        app.cache_update = CacheUpdate::Pending;
        assert_eq!(
            tap(&mut app, &mut config, &catalog, width * 3 / 4, y, false),
            ui::Action::None
        );
        assert_eq!(app.cache_update, CacheUpdate::Accepted);
        assert_eq!(app.screen, Screen::Walk);
        assert_eq!(app.draft, draft);
    }
}
