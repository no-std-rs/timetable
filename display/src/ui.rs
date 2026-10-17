//! Touch state machine. Editing remains provisional until Save is pressed.
use crate::{App, Config, Pick, Screen, catalog::Catalog, model::Status};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Save { refresh: bool },
}

pub fn swipe(app: &mut App, config: &Config, catalog: &Catalog<'_>, dx: i16, dy: i16) {
    if app.screen == Screen::Board && dx.unsigned_abs() > 35 {
        let n = config.groups.len();
        if n != 0 {
            app.choose_group((app.group + if dx < 0 { 1 } else { n - 1 }) % n, config);
        }
    } else if matches!(app.screen, Screen::Routes | Screen::Stops) && dy.unsigned_abs() > 35 {
        let count = if app.screen == Screen::Routes {
            catalog.route_count()
        } else {
            catalog
                .route_stops(app.selected_route, app.direction)
                .count()
        };
        app.cursor = if dy < 0 {
            app.cursor
                .saturating_add(4)
                .min(count.saturating_sub(1) / 4 * 4)
        } else {
            app.cursor.saturating_sub(4)
        };
    }
}

pub fn tap(
    app: &mut App,
    config: &mut Config,
    catalog: &Catalog<'_>,
    x: u16,
    y: u16,
    held: bool,
) -> Action {
    let Some(group) = config.groups.get_mut(app.group) else {
        return Action::None;
    };
    if app.screen != Screen::Board && y < 38 {
        app.screen = Screen::Board;
        app.draft = None;
        app.cursor = 0;
        return Action::None;
    }
    match app.screen {
        Screen::Board => {
            app.screen = if held || y > 190 {
                Screen::Picks
            } else {
                Screen::Groups
            };
            app.cursor = 0;
        }
        Screen::Groups => {
            if (52..196).contains(&y) {
                app.choose_group(((y - 52) / 36).into(), config);
            } else if y >= 200 {
                app.manual_until = 0;
                app.screen = Screen::Board;
            }
        }
        Screen::Picks => {
            if (55..195).contains(&y) {
                let index = ((y - 55) / 32) as usize;
                if let Some(pick) = group.picks.get(index) {
                    app.edit_pick = index;
                    app.draft = Some(pick.clone());
                    app.screen = Screen::Walk;
                } else if index == group.picks.len() && index < crate::config::MAX_PICKS {
                    app.edit_pick = index;
                    app.draft = None;
                    app.screen = Screen::Routes;
                    app.cursor = 0;
                }
            }
        }
        Screen::Routes => {
            if (58..186).contains(&y) {
                let index = app.cursor + ((y - 58) / 32) as usize;
                if catalog.route(index).is_some() {
                    app.selected_route = index;
                    app.direction = -1;
                    app.cursor = 0;
                    app.screen = Screen::Stops;
                }
            } else if y >= 195 {
                app.cursor = if x < 120 {
                    app.cursor.saturating_sub(4)
                } else {
                    (app.cursor + 4).min(catalog.route_count().saturating_sub(1) / 4 * 4)
                };
            }
        }
        Screen::Stops => {
            if (58..186).contains(&y) {
                if let Some((_, stop)) = catalog
                    .route_stops(app.selected_route, app.direction)
                    .nth(app.cursor + ((y - 58) / 32) as usize)
                {
                    let route = catalog.route(app.selected_route).unwrap();
                    let old = app.draft.as_ref();
                    let pick = Pick {
                        route: route.id.try_into().unwrap(),
                        line: route.line.try_into().unwrap(),
                        stop: stop.id.try_into().unwrap(),
                        stop_name: stop.name.try_into().unwrap(),
                        stop_code: stop.code.try_into().unwrap(),
                        headsign: if app.direction < 0 {
                            "Both directions"
                        } else {
                            route.headsigns[app.direction as usize]
                        }
                        .try_into()
                        .unwrap(),
                        direction: app.direction,
                        walk: old.map_or(5, |p| p.walk),
                        spread: old.map_or(1, |p| p.spread),
                        margin: old.map_or(1, |p| p.margin),
                    };
                    app.draft = Some(pick);
                    app.screen = Screen::Walk;
                }
            } else if y >= 195 {
                if (85..155).contains(&x) {
                    app.direction = if app.direction == 1 {
                        -1
                    } else {
                        app.direction + 1
                    };
                    app.cursor = 0;
                } else if x < 85 {
                    app.cursor = app.cursor.saturating_sub(4);
                } else {
                    let count = catalog
                        .route_stops(app.selected_route, app.direction)
                        .count();
                    app.cursor = (app.cursor + 4).min(count.saturating_sub(1) / 4 * 4);
                }
            }
        }
        Screen::Walk => {
            if y < 63 {
                app.screen = Screen::Routes;
                app.cursor = 0;
            } else if y < 177 {
                if let Some(p) = app.draft.as_mut() {
                    let (v, max) = if y < 101 {
                        (&mut p.walk, 60)
                    } else if y < 139 {
                        (&mut p.spread, 15)
                    } else {
                        (&mut p.margin, 10)
                    };
                    *v = if x < 120 {
                        v.saturating_sub(1)
                    } else {
                        v.saturating_add(1).min(max)
                    };
                }
            } else if y >= 190 {
                let mut changed = false;
                if x < 95 {
                    if app.edit_pick < group.picks.len() {
                        group.picks.remove(app.edit_pick);
                        changed = true;
                    }
                } else if let Some(pick) = app.draft.take() {
                    if let Some(old) = group.picks.get_mut(app.edit_pick) {
                        changed = old.route != pick.route
                            || old.stop != pick.stop
                            || old.direction != pick.direction;
                        *old = pick;
                    } else if group.picks.len() < crate::config::MAX_PICKS {
                        group.picks.push(pick).unwrap();
                        changed = true;
                    }
                }
                app.draft = None;
                app.screen = Screen::Board;
                if changed {
                    app.departures.clear();
                    app.loaded_at = None;
                    app.status = Status::Loading;
                }
                return Action::Save { refresh: changed };
            }
        }
    }
    Action::None
}
