//! Kintsugi palette, with one 240×16 tile on the board. No framebuffer or heap.
use crate::{
    App, Config, Screen,
    catalog::Catalog,
    clock,
    model::{Status, leave_in, walking_chance},
};
use core::fmt::Write;
use embedded_graphics::{
    mono_font::{
        MonoTextStyle,
        iso_8859_2::{FONT_6X10, FONT_10X20},
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, Line, PrimitiveStyle, Rectangle, RoundedRectangle},
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};
use heapless::String;

pub const PORCELAIN: Rgb565 = Rgb565::new(30, 59, 26);
pub const INK: Rgb565 = Rgb565::new(4, 9, 5);
pub const GOLD: Rgb565 = Rgb565::new(22, 37, 12);
pub const CHILLI: Rgb565 = Rgb565::new(23, 14, 6);
const MUTED: Rgb565 = Rgb565::new(15, 30, 14);

// The board redraws in tiles. Avoid rasterizing text and primitives that cannot
// touch this target; a full framebuffer still draws the same complete image.
fn draw_visible<D, T>(d: &mut D, item: &T) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
    T: Drawable<Color = Rgb565> + Dimensions,
{
    if !item
        .bounding_box()
        .intersection(&d.bounding_box())
        .is_zero_sized()
    {
        item.draw(d)?;
    }
    Ok(())
}

fn label<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    text: &str,
    x: i32,
    y: i32,
    color: Rgb565,
    center: bool,
) -> Result<(), D::Error> {
    draw_visible(
        d,
        &Text::with_text_style(
            text,
            Point::new(x, y),
            MonoTextStyle::new(&FONT_6X10, color),
            TextStyleBuilder::new()
                .baseline(Baseline::Top)
                .alignment(if center {
                    Alignment::Center
                } else {
                    Alignment::Left
                })
                .build(),
        ),
    )
}
fn short(value: &str, count: usize) -> String<96> {
    let mut out = String::new();
    for (i, c) in value.chars().enumerate() {
        if i == count {
            let _ = out.push_str("..");
            break;
        }
        let _ = out.push(c);
    }
    out
}
fn pill<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: Rgb565,
) -> Result<(), D::Error> {
    draw_visible(
        d,
        &RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(x, y), Size::new(w, h)),
            Size::new(5, 5),
        )
        .into_styled(PrimitiveStyle::with_fill(color)),
    )
}
fn big<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    value: &str,
    x: i32,
    y: i32,
) -> Result<(), D::Error> {
    struct Scale<'a, D> {
        d: &'a mut D,
        origin: Point,
    }
    impl<D> OriginDimensions for Scale<'_, D> {
        fn size(&self) -> Size {
            Size::new(120, 120)
        }
    }
    impl<D: DrawTarget<Color = Rgb565>> DrawTarget for Scale<'_, D> {
        type Color = Rgb565;
        type Error = D::Error;
        fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
            &mut self,
            pixels: I,
        ) -> Result<(), Self::Error> {
            for Pixel(p, c) in pixels {
                self.d
                    .fill_solid(&Rectangle::new(self.origin + p * 2, Size::new(2, 2)), c)?;
            }
            Ok(())
        }
    }
    let text = Text::with_baseline(
        value,
        Point::zero(),
        MonoTextStyle::new(&FONT_10X20, INK),
        Baseline::Top,
    );
    let bounds = text.bounding_box();
    let scaled = Rectangle::new(Point::new(x, y) + bounds.top_left * 2, bounds.size * 2);
    if scaled.intersection(&d.bounding_box()).is_zero_sized() {
        return Ok(());
    }
    text.draw(&mut Scale {
        d,
        origin: Point::new(x, y),
    })?;
    Ok(())
}

pub fn starting<D: DrawTarget<Color = Rgb565>>(d: &mut D) -> Result<(), D::Error> {
    background(d)?;
    label(d, "timetable", 120, 80, INK, true)?;
    label(d, "Getting ready...", 120, 120, MUTED, true)
}

pub fn setup<D: DrawTarget<Color = Rgb565>>(d: &mut D) -> Result<(), D::Error> {
    background(d)?;
    label(d, "timetable", 120, 60, INK, true)?;
    label(d, "A little time to spare.", 120, 89, MUTED, true)?;
    pill(d, 45, 125, 150, 28, GOLD)?;
    label(d, "Set up over USB", 120, 134, INK, true)?;
    label(d, "Wi-Fi + your groups", 120, 170, MUTED, true)
}

fn background<D: DrawTarget<Color = Rgb565>>(d: &mut D) -> Result<(), D::Error> {
    d.fill_solid(
        &Rectangle::new(Point::zero(), Size::new(240, 240)),
        PORCELAIN,
    )?;
    for (a, b) in [
        (Point::new(204, 21), Point::new(193, 37)),
        (Point::new(193, 37), Point::new(202, 49)),
        (Point::new(202, 49), Point::new(228, 56)),
    ] {
        draw_visible(
            d,
            &Line::new(a, b).into_styled(PrimitiveStyle::with_stroke(GOLD, 2)),
        )?;
    }
    Ok(())
}

pub fn draw<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    app: &App,
    config: &Config,
    catalog: &Catalog<'_>,
) -> Result<(), D::Error> {
    background(d)?;
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
    };
    label(d, title, 120, 24, INK, true)?;
    if app.screen != Screen::Board {
        label(d, "<", 45, 25, INK, false)?;
    }
    match app.screen {
        Screen::Board => {
            let now = app.clock.unwrap_or(0);
            let chosen = app
                .upcoming()
                .enumerate()
                .find(|(_, e)| {
                    config.groups[e.group as usize]
                        .picks
                        .get(e.pick as usize)
                        .is_some_and(|p| walking_chance(e.at, now, p) >= 50)
                })
                .or_else(|| app.upcoming().enumerate().next());
            if let Some((chosen_index, event)) = chosen {
                let p = &group.picks[event.pick as usize];
                pill(d, 32, 62, 47, 29, INK)?;
                draw_visible(
                    d,
                    &Text::with_text_style(
                        p.line.as_str(),
                        Point::new(55, 66),
                        MonoTextStyle::new(
                            if p.line.chars().count() > 4 {
                                &FONT_6X10
                            } else {
                                &FONT_10X20
                            },
                            PORCELAIN,
                        ),
                        TextStyleBuilder::new()
                            .baseline(Baseline::Top)
                            .alignment(Alignment::Center)
                            .build(),
                    ),
                )?;
                let mut value = String::<8>::new();
                let _ = write!(value, "{}", (event.at - now + 59) / 60);
                big(d, &value, 129, 55)?;
                label(d, "min", 197, 85, MUTED, false)?;
                label(
                    d,
                    &short(
                        app.headsigns
                            .get(event.headsign)
                            .filter(|s| !s.is_empty())
                            .unwrap_or(&p.headsign),
                        27,
                    ),
                    120,
                    96,
                    INK,
                    true,
                )?;
                label(d, &short(&p.stop_name, 27), 120, 108, MUTED, true)?;
                let left = leave_in(event.at, now, p);
                let mut text = String::<64>::new();
                if walking_chance(event.at, now, p) == 0 {
                    let _ = text.push_str("Not enough walk time");
                } else if left >= 60 {
                    let _ = write!(text, "Leave in {} min", left / 60);
                } else {
                    let _ = text.push_str("Leave now");
                }
                pill(d, 32, 123, 176, 25, if left < 60 { CHILLI } else { GOLD })?;
                label(
                    d,
                    &text,
                    120,
                    131,
                    if left < 60 { PORCELAIN } else { INK },
                    true,
                )?;
                text.clear();
                let _ = write!(
                    text,
                    "{}m walk / {}% walk est.{}",
                    p.walk,
                    walking_chance(event.at, now, p),
                    if event.conditional { " *" } else { "" }
                );
                label(d, &text, 120, 151, MUTED, true)?;
                for (i, next) in app
                    .upcoming()
                    .enumerate()
                    .filter(|(index, e)| e.at >= event.at && *index != chosen_index)
                    .map(|(_, event)| event)
                    .take(2)
                    .enumerate()
                {
                    let Some(p) = group.picks.get(next.pick as usize) else {
                        continue;
                    };
                    text.clear();
                    let _ = write!(
                        text,
                        "{} {}{}",
                        p.line,
                        app.headsigns
                            .get(next.headsign)
                            .filter(|s| !s.is_empty())
                            .unwrap_or(&p.headsign),
                        if next.conditional { " *" } else { "" }
                    );
                    label(d, &short(&text, 20), 38, 173 + i as i32 * 17, INK, false)?;
                    text.clear();
                    let _ = write!(text, "{}m", (next.at - now + 59) / 60);
                    label(d, &text, 182, 173 + i as i32 * 17, INK, false)?;
                }
            } else {
                let status = match app.status {
                    Status::Connecting => "Connecting to Wi-Fi",
                    Status::Syncing => "Setting the clock",
                    Status::Loading => "Reading the timetable",
                    Status::Offline => "Offline",
                    Status::Capacity => "Selection too large",
                    Status::Failed => "Update failed",
                    _ if group.picks.is_empty() => "Make this group yours",
                    _ => "No upcoming departures",
                };
                label(d, status, 120, 86, INK, true)?;
                label(
                    d,
                    if group.picks.is_empty() {
                        "Hold to add lines + stops"
                    } else if app.can_retry() {
                        "Tap below to retry"
                    } else {
                        "Scheduled times from ZTM"
                    },
                    120,
                    114,
                    MUTED,
                    true,
                )?;
                label(d, "Swipe to change group", 120, 158, MUTED, true)?;
            }
            let mut foot = String::<40>::new();
            let fault = app.can_retry() || app.status == Status::Capacity;
            if !fault && let Some(local) = clock::warsaw(now).filter(|_| app.clock.is_some()) {
                let _ = write!(foot, "{:02}:{:02} / ", local.hour(), local.minute());
            }
            let _ = foot.push_str(if app.save_failed {
                "NOT SAVED"
            } else if app.sleep_failed {
                "SLEEP FAILED"
            } else if app.status == Status::Offline {
                "OFFLINE / TAP RETRY"
            } else if app.status == Status::Failed {
                "FAILED / TAP RETRY"
            } else if app.status == Status::Capacity {
                "TOO MANY / HOLD EDIT"
            } else if app.status == Status::Connecting {
                "CONNECTING"
            } else if app.status == Status::Syncing {
                "SYNCING"
            } else if app.status == Status::Loading {
                "UPDATING"
            } else if now < app.manual_until
                || !config.automatic
                || !config.groups.iter().any(|g| g.days != 0)
            {
                "MANUAL"
            } else {
                "SCHEDULED"
            });
            label(
                d,
                &foot,
                120,
                212,
                if app.save_failed || app.sleep_failed || fault {
                    CHILLI
                } else {
                    MUTED
                },
                true,
            )?;
            if !fault {
                draw_visible(
                    d,
                    &Circle::new(Point::new(57, 214), 4).into_styled(PrimitiveStyle::with_fill(
                        if app.status == Status::Ready {
                            GOLD
                        } else {
                            CHILLI
                        },
                    )),
                )?;
            }
        }
        Screen::Groups => {
            for (i, g) in config.groups.iter().enumerate() {
                if i == app.group {
                    pill(d, 30, 50 + i as i32 * 36, 180, 31, GOLD)?;
                }
                label(d, &g.name, 120, 61 + i as i32 * 36, INK, true)?;
            }
            label(
                d,
                if config.automatic && config.groups.iter().any(|group| group.days != 0) {
                    "Return to auto"
                } else {
                    "Back to board"
                },
                120,
                207,
                MUTED,
                true,
            )?;
        }
        Screen::Picks => {
            for (i, p) in group.picks.iter().enumerate() {
                let mut row = String::<96>::new();
                let _ = write!(row, "{} {}", p.line, short(&p.stop_name, 22));
                label(d, &short(&row, 27), 32, 63 + i as i32 * 32, INK, false)?;
            }
            if group.picks.len() < crate::config::MAX_PICKS {
                label(
                    d,
                    "+ Add a line + stop",
                    32,
                    63 + group.picks.len() as i32 * 32,
                    CHILLI,
                    false,
                )?;
            }
            label(
                d,
                if app.save_failed {
                    "Save failed. Try again."
                } else {
                    "Tap to change your walk"
                },
                120,
                208,
                if app.save_failed { CHILLI } else { MUTED },
                true,
            )?;
        }
        Screen::Routes => {
            for i in 0..4 {
                if let Some(r) = catalog.route(app.cursor + i) {
                    if app
                        .draft
                        .as_ref()
                        .is_some_and(|pick| pick.route == r.id && pick.line == r.line)
                    {
                        pill(d, 25, 58 + i as i32 * 32, 190, 28, GOLD)?;
                    }
                    let mut row = String::<96>::new();
                    let _ = write!(row, "{} {}", r.line, short(r.headsigns[0], 24));
                    label(d, &short(&row, 27), 31, 66 + i as i32 * 32, INK, false)?;
                }
            }
            page_count(d, app.cursor, catalog.route_count())?;
            label(d, "< previous      next >", 120, 204, MUTED, true)?;
        }
        Screen::Stops => {
            let selected = crate::ui::selected_stop(app, catalog);
            for (i, (index, s)) in catalog
                .route_stops(app.selected_route, app.direction)
                .skip(app.cursor)
                .take(4)
                .enumerate()
            {
                if selected == Some(index) {
                    pill(d, 25, 57 + i as i32 * 32, 190, 29, GOLD)?;
                }
                label(d, &short(s.name, 27), 31, 61 + i as i32 * 32, INK, false)?;
                label(d, s.code, 31, 73 + i as i32 * 32, MUTED, false)?;
            }
            page_count(
                d,
                app.cursor,
                catalog
                    .route_stops(app.selected_route, app.direction)
                    .count(),
            )?;
            let route = catalog.route(app.selected_route);
            let mut foot = String::<32>::new();
            if app.direction < 0 {
                let _ = foot.push_str("Both");
            } else if let Some(name) = route
                .map(|route| route.headsigns[app.direction as usize])
                .filter(|name| !name.is_empty())
            {
                let _ = write!(foot, "{}", short(name, 8));
            } else {
                let _ = write!(foot, "Direction {}", app.direction + 1);
            }
            label(d, "<", 52, 204, MUTED, true)?;
            label(d, &foot, 120, 204, INK, true)?;
            label(d, ">", 188, 204, MUTED, true)?;
        }
        Screen::Walk => {
            if let Some(p) = &app.draft {
                let mut row = String::<96>::new();
                pill(d, 35, 43, 39, 20, INK)?;
                label(d, &short(&p.line, 4), 55, 48, PORCELAIN, true)?;
                label(d, &short(&p.stop_name, 17), 83, 48, INK, false)?;
                for (i, (title, value)) in [
                    ("Walk", p.walk),
                    ("Variation +/-", p.spread),
                    ("Boarding buffer", p.margin),
                ]
                .into_iter()
                .enumerate()
                {
                    row.clear();
                    let _ = write!(row, "-   {}: {}m   +", title, value);
                    label(d, &row, 120, 78 + i as i32 * 38, INK, true)?;
                }
                label(
                    d,
                    if app.edit_pick < group.picks.len() {
                        "Remove"
                    } else {
                        "Cancel"
                    },
                    70,
                    201,
                    CHILLI,
                    true,
                )?;
                pill(d, 117, 190, 73, 29, GOLD)?;
                label(d, "Save", 153, 200, INK, true)?;
            }
        }
    }
    Ok(())
}

fn page_count<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    cursor: usize,
    count: usize,
) -> Result<(), D::Error> {
    let mut text = String::<32>::new();
    if count == 0 {
        let _ = text.push_str("No stops in this direction");
    } else {
        let _ = write!(
            text,
            "{}-{} / {}",
            cursor + 1,
            (cursor + 4).min(count),
            count
        );
    }
    label(d, &text, 120, 185, MUTED, true)
}
