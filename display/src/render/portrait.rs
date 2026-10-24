//! Native 172×320 layout, shared by the C6 and browser. No scaling/framebuffer.
use super::*;

fn text<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    profile: crate::profile::Profile<'_>,
    value: &str,
    x: i32,
    y: i32,
    color: Rgb565,
    center: bool,
) -> Result<(), D::Error> {
    label_with_profile(profile, d, value, x, y, color, center)
}
fn base<D: DrawTarget<Color = Rgb565>>(d: &mut D) -> Result<(), D::Error> {
    d.fill_solid(
        &Rectangle::new(Point::zero(), Size::new(172, 320)),
        PORCELAIN,
    )?;
    for (a, b) in [
        (Point::new(153, 5), Point::new(145, 20)),
        (Point::new(145, 20), Point::new(163, 32)),
    ] {
        draw_visible(
            d,
            &Line::new(a, b).into_styled(PrimitiveStyle::with_stroke(GOLD, 2)),
        )?;
    }
    Ok(())
}
pub fn starting<D: DrawTarget<Color = Rgb565>>(d: &mut D) -> Result<(), D::Error> {
    base(d)?;
    label(d, "timetable", 86, 112, INK, true)?;
    label(d, "Getting ready...", 86, 140, MUTED, true)
}
pub fn setup<D: DrawTarget<Color = Rgb565>>(d: &mut D) -> Result<(), D::Error> {
    base(d)?;
    for (s, y, c) in [
        ("timetable", 34, INK),
        ("Your stops, one glance.", 60, MUTED),
        ("1 Open the installer", 100, INK),
        ("no-std-rs.github.io", 123, INK),
        ("/timetable", 137, CHILLI),
        ("2 Choose your stops", 180, INK),
        ("and walking time", 194, INK),
        ("3 Connect USB + install", 230, INK),
        ("Chrome / Edge", 270, INK),
        ("USB data cable", 288, MUTED),
    ] {
        label(d, s, 86, y, c, true)?;
    }
    Ok(())
}
pub fn draw_animated<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    app: &App,
    config: &Config,
    catalog: &Catalog<'_>,
    phase: u8,
) -> Result<(), D::Error> {
    base(d)?;
    let profile = catalog.profile();
    if app.cache_update == CacheUpdate::Pending {
        return cache_confirmation(d, profile, 172);
    }
    let Some(group) = config.groups.get(app.group) else {
        return setup(d);
    };
    if app.screen == Screen::Board && group.journey.is_some() {
        return journey_board(d, app, config, catalog, 172, 320);
    }
    let title = match app.screen {
        Screen::Board => group.name.as_str(),
        Screen::Groups => "Your groups",
        Screen::Picks => "Edit this group",
        Screen::Routes => "Choose a line",
        Screen::Stops => "Choose a stop",
        Screen::Walk => "Your walk",
        Screen::Freshness => "Timetable age",
    };
    text(d, profile, &short(title, 21), 86, 16, INK, true)?;
    if app.screen != Screen::Board {
        label(d, "<", 7, 16, INK, false)?;
    }
    match app.screen {
        Screen::Board => {
            let now = app.clock.unwrap_or(0);
            let status = if app.save_failed {
                "NOT SAVED"
            } else if app.sleep_failed {
                "SLEEP FAILED"
            } else {
                match app.status {
                    Status::Restoring => "OPENING SAVED DATA",
                    Status::Connecting => "CONNECTING",
                    Status::Syncing => "SYNCING CLOCK",
                    Status::Loading => "UPDATING",
                    Status::Offline => "OFFLINE / TAP RETRY",
                    Status::Failed => "FAILED / TAP RETRY",
                    Status::Capacity => "TOO MANY / HOLD EDIT",
                    _ if app.clock.is_none() => "SAVED / CLOCK PENDING",
                    _ => {
                        if config.automatic && now >= app.manual_until {
                            "SCHEDULED"
                        } else {
                            "MANUAL"
                        }
                    }
                }
            };
            let busy = matches!(
                app.status,
                Status::Restoring | Status::Connecting | Status::Syncing | Status::Loading
            );
            text(
                d,
                profile,
                status,
                86,
                39,
                if app.can_retry() { CHILLI } else { MUTED },
                true,
            )?;
            if busy {
                for i in 0..8 {
                    draw_visible(
                        d,
                        &Circle::new(Point::new(66 + i * 5, 54), 2).into_styled(
                            PrimitiveStyle::with_fill(if i == i32::from(phase % 8) {
                                CHILLI
                            } else {
                                GOLD
                            }),
                        ),
                    )?;
                }
            }
            if let Some((index, event)) = app.featured(config) {
                let pick = &group.picks[event.pick as usize];
                pill(d, 8, 65, 43, 23, INK)?;
                text(d, profile, &short(&pick.line, 6), 29, 72, PORCELAIN, true)?;
                text(
                    d,
                    profile,
                    if event.conditional {
                        "ON REQUEST"
                    } else if app.clock.is_some() {
                        "DEPARTS IN"
                    } else {
                        "SAVED TIME"
                    },
                    103,
                    72,
                    MUTED,
                    true,
                )?;
                let value = if app.clock.is_some() {
                    countdown(event.at - now)
                } else {
                    departure_time(event.at, profile.zone())
                };
                big(d, &value, 86 - value.len() as i32 * 10, 94)?;
                let head = app
                    .headsigns
                    .get(event.headsign)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&pick.headsign);
                text(d, profile, &short(head, 26), 86, 140, INK, true)?;
                let mut stop = short(&pick.stop_name, 20);
                if !pick.stop_code.is_empty() {
                    let _ = write!(stop, " / {}", pick.stop_code);
                }
                text(d, profile, &short(&stop, 26), 86, 154, MUTED, true)?;
                let mut walk = String::<64>::new();
                if app.clock.is_some() {
                    let leave = leave_in(event.at, now, pick);
                    if leave > 0 {
                        let _ = write!(walk, "Leave in {}m", (leave + 59) / 60);
                    } else {
                        let _ = walk.push_str("Leave now");
                    }
                    let _ = write!(walk, " / {}m walk", pick.walk);
                } else {
                    let _ = walk.push_str("Waiting for NTP");
                }
                text(d, profile, &walk, 86, 173, CHILLI, true)?;
                if app.clock.is_some() {
                    walk.clear();
                    let _ = write!(walk, "{}% walk est.", walking_chance(event.at, now, pick));
                    text(d, profile, &walk, 86, 186, MUTED, true)?;
                }
                let mut n = 0;
                for (i, event) in app.departures.iter().enumerate() {
                    if i == index
                        || event.group as usize != app.group
                        || app.clock.is_some_and(|now| event.at < now)
                    {
                        continue;
                    }
                    if n == 3 {
                        break;
                    }
                    let pick = &group.picks[event.pick as usize];
                    let y = 204 + n * 24;
                    let mut row = String::<64>::new();
                    let _ = write!(
                        row,
                        "{}  {}{}",
                        short(&pick.line, 6),
                        departure_time(event.at, profile.zone()),
                        if event.conditional { " *" } else { "" }
                    );
                    text(d, profile, &row, 8, y, INK, false)?;
                    let head = app
                        .headsigns
                        .get(event.headsign)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&pick.headsign);
                    text(d, profile, &short(head, 26), 8, y + 11, MUTED, false)?;
                    n += 1;
                }
            } else {
                text(
                    d,
                    profile,
                    if app.loaded_at.is_some() {
                        "No upcoming departures"
                    } else {
                        "Your stops are saved"
                    },
                    86,
                    94,
                    INK,
                    true,
                )?;
                for (i, pick) in group.picks.iter().enumerate() {
                    let y = 120 + i as i32 * 32;
                    let mut row = String::<64>::new();
                    let _ = write!(row, "{} {}", pick.line, short(&pick.stop_name, 19));
                    text(d, profile, &short(&row, 26), 8, y, INK, false)?;
                    text(
                        d,
                        profile,
                        &short(&pick.headsign, 26),
                        8,
                        y + 12,
                        MUTED,
                        false,
                    )?;
                }
                if app.loaded_at.is_none() {
                    text(
                        d,
                        profile,
                        "First update takes longer",
                        86,
                        260,
                        MUTED,
                        true,
                    )?;
                }
            }
            text(
                d,
                profile,
                &short(&freshness_in_zone(app, profile.zone()), 27),
                86,
                283,
                MUTED,
                true,
            )?;
            let mut foot = String::<48>::new();
            if let Some(local) = profile.zone().local(now).filter(|_| app.clock.is_some()) {
                let _ = write!(
                    foot,
                    "{:02}:{:02}:{:02} / ",
                    local.hour(),
                    local.minute(),
                    local.second()
                );
            }
            let _ = foot.push_str("hold to edit");
            text(d, profile, &foot, 86, 302, MUTED, true)?;
        }
        Screen::Freshness => {
            for (title, value, y) in [
                (
                    "Last successful check",
                    timestamp(app.freshness.checked_at, "Not checked yet", profile.zone()),
                    78,
                ),
                (
                    "Source last updated",
                    timestamp(
                        app.freshness.source_updated_at,
                        "Not supplied",
                        profile.zone(),
                    ),
                    148,
                ),
            ] {
                text(d, profile, title, 86, y, MUTED, true)?;
                text(d, profile, &value, 86, y + 20, INK, true)?;
            }
            text(d, profile, "Oldest source file used", 86, 194, MUTED, true)?;
            text(
                d,
                profile,
                &short(profile.timezone().unwrap_or("Local time"), 26),
                86,
                226,
                MUTED,
                true,
            )?;
            text(
                d,
                profile,
                if app.can_retry() {
                    "Retry now"
                } else {
                    "Tap to return"
                },
                86,
                288,
                CHILLI,
                true,
            )?;
        }
        Screen::Groups => {
            for (i, g) in config.groups.iter().enumerate() {
                let y = 56 + i as i32 * 48;
                if i == app.group {
                    pill(d, 6, y, 160, 42, GOLD)?;
                }
                text(d, profile, &short(&g.name, 24), 86, y + 16, INK, true)?;
            }
            text(
                d,
                profile,
                if config.automatic {
                    "Return to auto"
                } else {
                    "Back to board"
                },
                86,
                288,
                MUTED,
                true,
            )?;
        }
        Screen::Picks => {
            for (i, p) in group.picks.iter().enumerate() {
                let y = 56 + i as i32 * 48;
                let mut row = String::<64>::new();
                let _ = write!(row, "{} {}", p.line, short(&p.stop_name, 20));
                text(d, profile, &short(&row, 26), 8, y + 8, INK, false)?;
                text(d, profile, &short(&p.headsign, 26), 8, y + 23, MUTED, false)?;
            }
            if group.picks.len() < crate::config::MAX_PICKS {
                text(
                    d,
                    profile,
                    "+ Add a line + stop",
                    8,
                    64 + group.picks.len() as i32 * 48,
                    CHILLI,
                    false,
                )?;
            }
            text(
                d,
                profile,
                if app.save_failed {
                    "Save failed. Try again."
                } else {
                    "Tap to change your walk"
                },
                86,
                288,
                MUTED,
                true,
            )?;
        }
        Screen::Routes | Screen::Stops => {
            let selected = crate::ui::selected_stop(app, catalog);
            for i in 0..4 {
                let y = 56 + i as i32 * 48;
                let (first, second, highlight) = if app.screen == Screen::Routes {
                    let Some(r) = catalog.route(app.cursor + i) else {
                        break;
                    };
                    (
                        short(r.line, 26),
                        short(r.headsigns[0], 26),
                        app.draft
                            .as_ref()
                            .is_some_and(|p| p.route == r.id && p.line == r.line),
                    )
                } else {
                    let Some((index, s)) = catalog
                        .route_stops(app.selected_route, app.direction)
                        .nth(app.cursor + i)
                    else {
                        break;
                    };
                    (
                        short(s.name, 26),
                        short(s.code, 26),
                        selected == Some(index),
                    )
                };
                if highlight {
                    pill(d, 4, y, 164, 42, GOLD)?;
                }
                text(d, profile, &first, 8, y + 7, INK, false)?;
                text(d, profile, &second, 8, y + 22, MUTED, false)?;
            }
            let count = if app.screen == Screen::Routes {
                catalog.route_count()
            } else {
                catalog
                    .route_stops(app.selected_route, app.direction)
                    .count()
            };
            let mut page = String::<32>::new();
            let _ = write!(
                page,
                "{}-{} / {}",
                (app.cursor + 1).min(count),
                (app.cursor + 4).min(count),
                count
            );
            text(d, profile, &page, 86, 258, MUTED, true)?;
            let mut direction = String::<32>::new();
            if app.screen == Screen::Stops {
                if app.direction < 0 {
                    let _ = direction.push_str("Both directions");
                } else if let Some(r) = catalog.route(app.selected_route) {
                    let _ = write!(
                        direction,
                        "{}",
                        short(r.headsigns[app.direction as usize], 16)
                    );
                }
            } else {
                let _ = direction.push_str("Page");
            }
            text(d, profile, "<", 10, 288, INK, false)?;
            text(d, profile, &direction, 86, 288, INK, true)?;
            text(d, profile, ">", 156, 288, INK, false)?;
        }
        Screen::Walk => {
            if let Some(p) = &app.draft {
                let mut row = String::<64>::new();
                let _ = write!(row, "{} {}", p.line, short(&p.stop_name, 20));
                text(d, profile, &short(&row, 26), 86, 50, INK, true)?;
                text(d, profile, "Tap to change line / stop", 86, 66, MUTED, true)?;
                for (i, (name, value)) in [
                    ("Walk", p.walk),
                    ("Variation +/-", p.spread),
                    ("Boarding buffer", p.margin),
                ]
                .into_iter()
                .enumerate()
                {
                    let y = 96 + i as i32 * 48;
                    text(d, profile, name, 86, y, MUTED, true)?;
                    row.clear();
                    let _ = write!(row, "-       {}m       +", value);
                    text(d, profile, &row, 86, y + 16, INK, true)?;
                }
                text(
                    d,
                    profile,
                    if app.edit_pick < group.picks.len() {
                        "Remove"
                    } else {
                        "Cancel"
                    },
                    40,
                    288,
                    CHILLI,
                    true,
                )?;
                pill(d, 85, 276, 80, 34, GOLD)?;
                text(d, profile, "Save", 125, 288, INK, true)?;
            }
        }
    }
    Ok(())
}
