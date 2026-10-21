//! Native 320×172 C6 layout. Full-size text and controls, no scaled framebuffer.
use super::*;

fn text<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    p: crate::profile::Profile<'_>,
    value: &str,
    x: i32,
    y: i32,
    color: Rgb565,
    center: bool,
) -> Result<(), D::Error> {
    label_with_profile(p, d, value, x, y, color, center)
}
fn base<D: DrawTarget<Color = Rgb565>>(d: &mut D) -> Result<(), D::Error> {
    d.fill_solid(
        &Rectangle::new(Point::zero(), Size::new(320, 172)),
        PORCELAIN,
    )?;
    for (a, b) in [
        (Point::new(300, 4), Point::new(292, 15)),
        (Point::new(292, 15), Point::new(312, 25)),
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
    label(d, "timetable", 160, 58, INK, true)?;
    label(d, "Getting ready...", 160, 84, MUTED, true)
}
pub fn setup<D: DrawTarget<Color = Rgb565>>(d: &mut D) -> Result<(), D::Error> {
    base(d)?;
    for (s, y, c) in [
        ("timetable / your stops, one glance", 16, INK),
        ("1 Open no-std-rs.github.io/timetable", 50, INK),
        ("2 Choose your stops and walking time", 76, INK),
        ("3 Connect USB and install", 102, INK),
        ("Chrome / Edge / USB data cable", 142, MUTED),
    ] {
        label(d, s, 160, y, c, true)?;
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
    let Some(group) = config.groups.get(app.group) else {
        return setup(d);
    };
    let title = match app.screen {
        Screen::Board => group.name.as_str(),
        Screen::Groups => "Your groups",
        Screen::Picks => "Edit this group",
        Screen::Routes => "Choose a line",
        Screen::Stops => "Choose a stop",
        Screen::Walk => "Your walk",
        Screen::Freshness => "Timetable age",
    };
    text(d, profile, &short(title, 40), 160, 8, INK, true)?;
    if app.screen != Screen::Board {
        label(d, "<", 8, 8, INK, false)?;
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
            text(
                d,
                profile,
                status,
                160,
                24,
                if app.can_retry() { CHILLI } else { MUTED },
                true,
            )?;
            if matches!(
                app.status,
                Status::Restoring | Status::Connecting | Status::Syncing | Status::Loading
            ) {
                for i in 0..8 {
                    draw_visible(
                        d,
                        &Circle::new(Point::new(8 + i * 3, 26), 2).into_styled(
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
                pill(d, 8, 42, 45, 23, INK)?;
                text(d, profile, &short(&pick.line, 6), 30, 49, PORCELAIN, true)?;
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
                    105,
                    49,
                    MUTED,
                    true,
                )?;
                let value = if app.clock.is_some() {
                    countdown(event.at - now)
                } else {
                    departure_time(event.at, profile.zone())
                };
                big(d, &value, 80 - value.len() as i32 * 10, 68)?;
                let head = app
                    .headsigns
                    .get(event.headsign)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&pick.headsign);
                text(d, profile, &fit_label(head, 144, 1), 80, 110, INK, true)?;
                let mut stop = String::<96>::new();
                if !pick.stop_code.is_empty() {
                    let _ = write!(stop, "{} / ", pick.stop_code);
                }
                let _ = stop.push_str(&pick.stop_name);
                text(d, profile, &fit_label(&stop, 152, 1), 80, 123, MUTED, true)?;
                let mut walk = String::<64>::new();
                if app.clock.is_some() {
                    let chance = walking_chance(event.at, now, pick);
                    let left = leave_in(event.at, now, pick);
                    if chance == 0 {
                        let _ = walk.push_str("Too late to walk");
                    } else if left > 0 {
                        let _ = write!(walk, "Leave in {}m", (left + 59) / 60);
                    } else {
                        let _ = walk.push_str("Leave now");
                    }
                    let _ = write!(walk, " / {}m / {}% est.", pick.walk, chance);
                } else if let Some(saved) = app.loaded_at.and_then(|at| profile.zone().local(at)) {
                    let _ = write!(
                        walk,
                        "Saved {:02}/{:02} / clock pending",
                        saved.day(),
                        saved.month() as u8
                    );
                } else {
                    let _ = walk.push_str("Saved times / clock pending");
                }
                text(d, profile, &walk, 160, 139, CHILLI, true)?;
                let mut n = 0;
                for (i, event) in app.departures.iter().enumerate() {
                    if i == index
                        || event.group as usize != app.group
                        || app.clock.is_some_and(|now| event.at < now)
                    {
                        continue;
                    }
                    if n == 4 {
                        break;
                    }
                    let pick = &group.picks[event.pick as usize];
                    let y = 42 + n * 23;
                    let mut row = String::<64>::new();
                    let _ = write!(
                        row,
                        "{} {} {}{}",
                        short(&pick.line, 6),
                        departure_time(event.at, profile.zone()),
                        short(&pick.stop_code, 8),
                        if event.conditional { " *" } else { "" }
                    );
                    text(d, profile, &fit_label(&row, 144, 1), 168, y, INK, false)?;
                    let head = app
                        .headsigns
                        .get(event.headsign)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&pick.headsign);
                    text(
                        d,
                        profile,
                        &fit_label(head, 144, 1),
                        168,
                        y + 11,
                        MUTED,
                        false,
                    )?;
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
                    160,
                    46,
                    INK,
                    true,
                )?;
                for (i, pick) in group.picks.iter().enumerate() {
                    let mut row = String::<80>::new();
                    let _ = write!(
                        row,
                        "{}  {} / {}",
                        pick.line,
                        short(&pick.stop_name, 24),
                        short(&pick.headsign, 16)
                    );
                    text(
                        d,
                        profile,
                        &short(&row, 49),
                        8,
                        66 + i as i32 * 16,
                        INK,
                        false,
                    )?;
                }
                if app.loaded_at.is_none() {
                    text(
                        d,
                        profile,
                        "First update takes longer; later boots use cache",
                        160,
                        137,
                        MUTED,
                        true,
                    )?;
                }
            }
            draw_visible(
                d,
                &Line::new(Point::new(8, 153), Point::new(312, 153))
                    .into_styled(PrimitiveStyle::with_stroke(GOLD, 1)),
            )?;
            let mut clock = String::<32>::new();
            if let Some(local) = profile.zone().local(now).filter(|_| app.clock.is_some()) {
                let _ = write!(
                    clock,
                    "{:02}:{:02}:{:02}",
                    local.hour(),
                    local.minute(),
                    local.second()
                );
            } else {
                let _ = clock.push_str("CLOCK PENDING");
            }
            text(d, profile, &clock, 8, 160, MUTED, false)?;
            text(
                d,
                profile,
                &freshness_in_zone(app, profile.zone()),
                235,
                160,
                MUTED,
                true,
            )?;
        }
        Screen::Freshness => {
            for (title, value, x) in [
                (
                    "Last successful check",
                    timestamp(app.freshness.checked_at, "Not checked yet", profile.zone()),
                    80,
                ),
                (
                    "Source last updated",
                    timestamp(
                        app.freshness.source_updated_at,
                        "Not supplied",
                        profile.zone(),
                    ),
                    240,
                ),
            ] {
                text(d, profile, title, x, 55, MUTED, true)?;
                text(d, profile, &value, x, 78, INK, true)?;
            }
            text(
                d,
                profile,
                profile.timezone().unwrap_or("Local time"),
                160,
                112,
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
                160,
                155,
                CHILLI,
                true,
            )?;
        }
        Screen::Groups => {
            for (i, g) in config.groups.iter().enumerate() {
                let x = 8 + (i % 2) as i32 * 160;
                let y = 33 + (i / 2) as i32 * 48;
                pill(
                    d,
                    x,
                    y,
                    144,
                    46,
                    if i == app.group {
                        GOLD
                    } else {
                        Rgb565::new(28, 56, 24)
                    },
                )?;
                large(d, profile, &g.name, x + 6, y + 13, 132, INK)?;
            }
            text(
                d,
                profile,
                if config.automatic {
                    "Return to auto"
                } else {
                    "Back to board"
                },
                160,
                148,
                INK,
                true,
            )?;
        }
        Screen::Picks => {
            for i in 0..4 {
                let x = 8 + (i % 2) as i32 * 160;
                let y = 33 + (i / 2) as i32 * 48;
                if let Some(p) = group.picks.get(i) {
                    pill(d, x, y, 144, 46, Rgb565::new(28, 56, 24))?;
                    large(d, profile, &p.line, x + 6, y + 1, 132, INK)?;
                    text(
                        d,
                        profile,
                        &fit_label(&p.stop_name, 132, 1),
                        x + 6,
                        y + 23,
                        MUTED,
                        false,
                    )?;
                    text(
                        d,
                        profile,
                        &fit_label(&p.headsign, 132, 1),
                        x + 6,
                        y + 34,
                        MUTED,
                        false,
                    )?;
                } else if i == group.picks.len() {
                    pill(d, x, y, 144, 46, GOLD)?;
                    large(d, profile, "+ Add", x + 6, y + 3, 132, INK)?;
                    text(d, profile, "Choose line + stop", x + 6, y + 30, INK, false)?;
                }
            }
            text(
                d,
                profile,
                if app.save_failed {
                    "Save failed / Back"
                } else {
                    "Back to board"
                },
                160,
                148,
                if app.save_failed { CHILLI } else { INK },
                true,
            )?;
        }
        Screen::Routes | Screen::Stops => {
            let selected = crate::ui::selected_stop(app, catalog);
            for i in 0..2 {
                let y = 33 + i as i32 * 48;
                let (first, second, highlight) = if app.screen == Screen::Routes {
                    let Some(r) = catalog.route(app.cursor + i) else {
                        break;
                    };
                    let mut title = String::<96>::new();
                    let _ = write!(title, "{}  {}", r.line, r.headsigns[0]);
                    (
                        title,
                        short(r.id, 48),
                        app.draft
                            .as_ref()
                            .is_some_and(|p| p.route == r.id && p.line == r.line),
                    )
                } else {
                    let Some((index, stop)) = catalog
                        .route_stops(app.selected_route, app.direction)
                        .nth(app.cursor + i)
                    else {
                        break;
                    };
                    (
                        short(stop.name, 48),
                        short(stop.code, 48),
                        selected == Some(index),
                    )
                };
                pill(
                    d,
                    4,
                    y,
                    312,
                    46,
                    if highlight {
                        GOLD
                    } else {
                        Rgb565::new(28, 56, 24)
                    },
                )?;
                large(d, profile, &first, 10, y + 4, 300, INK)?;
                // Route IDs are implementation details. Show boarding platform codes only.
                if app.screen == Screen::Stops {
                    text(d, profile, &second, 10, y + 29, MUTED, false)?;
                }
            }
            let count = if app.screen == Screen::Routes {
                catalog.route_count()
            } else {
                catalog
                    .route_stops(app.selected_route, app.direction)
                    .count()
            };
            let mut foot = String::<48>::new();
            if app.screen == Screen::Stops {
                if app.direction < 0 {
                    let _ = foot.push_str("Both directions");
                } else if let Some(r) = catalog.route(app.selected_route) {
                    let _ = write!(foot, "{}", short(r.headsigns[app.direction as usize], 26));
                }
            } else {
                let _ = write!(
                    foot,
                    "{}-{} / {}",
                    (app.cursor + 1).min(count),
                    (app.cursor + 2).min(count),
                    count
                );
            }
            pill(d, 4, 130, 60, 40, GOLD)?;
            pill(d, 256, 130, 60, 40, GOLD)?;
            large(d, profile, "<", 26, 140, 20, INK)?;
            large(d, profile, ">", 278, 140, 20, INK)?;
            text(d, profile, &foot, 160, 140, INK, true)?;
            text(
                d,
                profile,
                if app.screen == Screen::Stops {
                    "Tap direction / swipe to page"
                } else {
                    "Swipe or tap arrows"
                },
                160,
                156,
                MUTED,
                true,
            )?;
        }
        Screen::Walk => {
            if let Some(p) = &app.draft {
                large(d, profile, &p.line, 8, 34, 52, INK)?;
                large(d, profile, &p.stop_name, 70, 34, 242, INK)?;
                for (i, (name, value)) in [
                    ("Walk", p.walk),
                    ("Variation +/-", p.spread),
                    ("Boarding buffer", p.margin),
                ]
                .into_iter()
                .enumerate()
                {
                    let x = 4 + i as i32 * 106;
                    pill(d, x, 62, 100, 64, Rgb565::new(28, 56, 24))?;
                    text(d, profile, name, x + 50, 67, MUTED, true)?;
                    let mut value_text = String::<24>::new();
                    let _ = write!(value_text, "- {}m +", value);
                    large(d, profile, &value_text, x + 8, 94, 88, INK)?;
                }
                pill(d, 4, 130, 112, 40, PORCELAIN)?;
                pill(d, 204, 130, 112, 40, GOLD)?;
                text(
                    d,
                    profile,
                    if app.edit_pick < group.picks.len() {
                        "Remove"
                    } else {
                        "Cancel"
                    },
                    60,
                    147,
                    CHILLI,
                    true,
                )?;
                text(d, profile, "Save", 260, 147, INK, true)?;
            }
        }
    }
    Ok(())
}

/// Double the profile's small glyphs for legible touch choices, retaining the
/// same caller-owned tile. Truncate by rendered width, including non-Latin glyphs.
fn large<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    profile: crate::profile::Profile<'_>,
    value: &str,
    x: i32,
    y: i32,
    width: u32,
    color: Rgb565,
) -> Result<(), D::Error> {
    struct Double<'a, D> {
        d: &'a mut D,
        origin: Point,
    }
    impl<D: Dimensions> Dimensions for Double<'_, D> {
        fn bounding_box(&self) -> Rectangle {
            let b = self.d.bounding_box();
            let p = b.top_left - self.origin;
            Rectangle::new(
                Point::new(p.x.div_euclid(2), p.y.div_euclid(2)),
                Size::new(b.size.width / 2 + 2, b.size.height / 2 + 2),
            )
        }
    }
    impl<D: DrawTarget<Color = Rgb565>> DrawTarget for Double<'_, D> {
        type Color = Rgb565;
        type Error = D::Error;
        fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
            &mut self,
            pixels: I,
        ) -> Result<(), D::Error> {
            for Pixel(p, c) in pixels {
                self.d
                    .fill_solid(&Rectangle::new(self.origin + p * 2, Size::new(2, 2)), c)?;
            }
            Ok(())
        }
    }
    let fit = fit_label(value, width, 2);
    label_with_profile(
        profile,
        &mut Double {
            d,
            origin: Point::new(x, y),
        },
        &fit,
        0,
        0,
        color,
        false,
    )
}

fn fit_label(value: &str, width: u32, scale: u32) -> String<96> {
    let glyph_width = |c: char| {
        if !c.is_ascii()
            && c != '?'
            && FONT_6X10.glyph_mapping.index(c) == FONT_6X10.glyph_mapping.index('?')
        {
            10 * scale
        } else {
            6 * scale
        }
    };
    let mut fit = String::<96>::new();
    let mut used = 0;
    for c in value.chars() {
        let w = glyph_width(c);
        if used + w > width {
            while used + 18 * scale > width {
                let Some(last) = fit.pop() else {
                    break;
                };
                used -= glyph_width(last);
            }
            let _ = fit.push_str("...");
            break;
        }
        if fit.push(c).is_err() {
            break;
        }
        used += w;
    }
    fit
}
