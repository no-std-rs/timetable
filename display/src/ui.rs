//! Touch state machine. Editing remains provisional until Save is pressed.
use crate::{App, Config, Pick, Screen, catalog::Catalog, model::Status};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Refresh,
    Save { refresh: bool },
}

fn draft_route(app: &App, catalog: &Catalog<'_>) -> Option<usize> {
    let pick = app.draft.as_ref()?;
    (0..catalog.route_count()).find(|&i| {
        catalog
            .route(i)
            .is_some_and(|route| route.id == pick.route && route.line == pick.line)
    })
}

pub(crate) fn selected_stop(app: &App, catalog: &Catalog<'_>) -> Option<usize> {
    let pick = app.draft.as_ref()?;
    let mut found = None;
    let mut ambiguous = false;
    for (index, stop) in catalog.route_stops(app.selected_route, app.direction) {
        let matches = if pick.stop_code.is_empty() {
            stop.id == pick.stop && stop.name == pick.stop_name
        } else {
            stop.code == pick.stop_code
        };
        if matches {
            if stop.id == pick.stop {
                return Some(index);
            }
            ambiguous |= found.is_some();
            found = Some(index);
        }
    }
    if ambiguous { None } else { found }
}

fn stop_page(app: &App, catalog: &Catalog<'_>) -> usize {
    selected_stop(app, catalog)
        .and_then(|selected| {
            catalog
                .route_stops(app.selected_route, app.direction)
                .position(|(index, _)| index == selected)
        })
        .unwrap_or(0)
        / 4
        * 4
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
        app.cursor = 0;
        app.screen = match app.screen {
            Screen::Stops => {
                app.cursor = app.selected_route / 4 * 4;
                Screen::Routes
            }
            Screen::Routes if app.draft.is_some() => Screen::Walk,
            Screen::Routes => Screen::Picks,
            Screen::Walk => {
                app.draft = None;
                Screen::Picks
            }
            _ => Screen::Board,
        };
        return Action::None;
    }
    match app.screen {
        Screen::Board => {
            if !held && y > 190 && app.can_retry() {
                app.status = if app.status == Status::Offline {
                    Status::Connecting
                } else {
                    Status::Loading
                };
                return Action::Refresh;
            } else if held || y > 190 {
                app.screen = Screen::Picks;
            } else if y < 50 {
                app.screen = Screen::Groups;
            }
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
                if let Some(route) = catalog.route(index) {
                    app.selected_route = index;
                    app.direction = app
                        .draft
                        .as_ref()
                        .filter(|pick| pick.route == route.id && pick.line == route.line)
                        .map_or(-1, |pick| pick.direction);
                    app.cursor = stop_page(app, catalog);
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
                    for _ in 0..3 {
                        app.direction = if app.direction == 1 {
                            -1
                        } else {
                            app.direction + 1
                        };
                        if app.direction < 0
                            || catalog
                                .route_stops(app.selected_route, app.direction)
                                .next()
                                .is_some()
                        {
                            break;
                        }
                    }
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
                let route = draft_route(app, catalog);
                app.selected_route = route.unwrap_or(0);
                app.direction = app.draft.as_ref().map_or(-1, |pick| pick.direction);
                if x >= 82 && route.is_some() {
                    app.screen = Screen::Stops;
                    app.cursor = stop_page(app, catalog);
                } else {
                    app.screen = Screen::Routes;
                    app.cursor = app.selected_route / 4 * 4;
                }
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
                    } else {
                        app.draft = None;
                        app.screen = Screen::Picks;
                        return Action::None;
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
