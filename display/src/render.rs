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

fn label<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    text: &str,
    x: i32,
    y: i32,
    color: Rgb565,
    center: bool,
) -> Result<(), D::Error> {
    Text::with_text_style(
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
    )
    .draw(d)?;
    Ok(())
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
    RoundedRectangle::with_equal_corners(
        Rectangle::new(Point::new(x, y), Size::new(w, h)),
        Size::new(5, 5),
    )
    .into_styled(PrimitiveStyle::with_fill(color))
    .draw(d)?;
    Ok(())
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
    Text::with_baseline(
        value,
        Point::zero(),
        MonoTextStyle::new(&FONT_10X20, INK),
        Baseline::Top,
    )
    .draw(&mut Scale {
        d,
        origin: Point::new(x, y),
    })?;
    Ok(())
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
        Line::new(a, b)
            .into_styled(PrimitiveStyle::with_stroke(GOLD, 2))
            .draw(d)?;
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
        label(d, "<", 45, 25, GOLD, false)?;
    }
    match app.screen {
        Screen::Board => {
            let now = app.clock.unwrap_or(0);
            let chosen = app
                .upcoming()
                .find(|e| {
                    config.groups[e.group as usize]
                        .picks
                        .get(e.pick as usize)
                        .is_some_and(|p| walking_chance(e.at, now, p) >= 50)
                })
                .or_else(|| app.upcoming().next());
            if let Some(event) = chosen {
                let p = &group.picks[event.pick as usize];
                pill(d, 32, 62, 47, 29, INK)?;
                Text::with_text_style(
                    p.line.as_str(),
                    Point::new(55, 66),
                    MonoTextStyle::new(&FONT_10X20, PORCELAIN),
                    TextStyleBuilder::new()
                        .baseline(Baseline::Top)
                        .alignment(Alignment::Center)
                        .build(),
                )
                .draw(d)?;
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
                if left >= 60 {
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
                    .filter(|e| e.at >= event.at && !core::ptr::eq(*e, event))
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
            if let Some(local) = clock::warsaw(now).filter(|_| app.clock.is_some()) {
                let _ = write!(foot, "{:02}:{:02} / ", local.hour(), local.minute());
            }
            let _ = foot.push_str(if app.save_failed {
                "NOT SAVED"
            } else if matches!(
                app.status,
                Status::Offline | Status::Failed | Status::Capacity
            ) {
                "OFFLINE"
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
                if app.save_failed { CHILLI } else { MUTED },
                true,
            )?;
            Circle::new(Point::new(57, 214), 4)
                .into_styled(PrimitiveStyle::with_fill(if app.status == Status::Ready {
                    GOLD
                } else {
                    CHILLI
                }))
                .draw(d)?;
        }
        Screen::Groups => {
            for (i, g) in config.groups.iter().enumerate() {
                if i == app.group {
                    pill(d, 30, 50 + i as i32 * 36, 180, 31, GOLD)?;
                }
                label(d, &g.name, 120, 61 + i as i32 * 36, INK, true)?;
            }
            label(d, "Return to auto", 120, 207, MUTED, true)?;
        }
        Screen::Picks => {
            for (i, p) in group.picks.iter().enumerate() {
                let mut row = String::<96>::new();
                let _ = write!(row, "{} {}", p.line, short(&p.stop_name, 22));
                label(d, &row, 32, 63 + i as i32 * 32, INK, false)?;
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
                    let mut row = String::<96>::new();
                    let _ = write!(row, "{} {}", r.line, short(r.headsigns[0], 24));
                    label(d, &row, 31, 66 + i as i32 * 32, INK, false)?;
                }
            }
            label(d, "< previous      next >", 120, 204, MUTED, true)?;
        }
        Screen::Stops => {
            for (i, (_, s)) in catalog
                .route_stops(app.selected_route, app.direction)
                .skip(app.cursor)
                .take(4)
                .enumerate()
            {
                label(d, &short(s.name, 27), 31, 61 + i as i32 * 32, INK, false)?;
                label(d, s.code, 31, 73 + i as i32 * 32, MUTED, false)?;
            }
            let direction = match app.direction {
                0 => "0",
                1 => "1",
                _ => "any",
            };
            let mut foot = String::<32>::new();
            let _ = write!(foot, "<     dir {}     >", direction);
            label(d, &foot, 120, 204, MUTED, true)?;
        }
        Screen::Walk => {
            if let Some(p) = &app.draft {
                let mut row = String::<96>::new();
                let _ = write!(row, "{} {}", p.line, short(&p.stop_name, 22));
                label(d, &row, 120, 47, GOLD, true)?;
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
                label(d, "Remove", 70, 201, CHILLI, true)?;
                pill(d, 117, 190, 73, 29, GOLD)?;
                label(d, "Save", 153, 200, INK, true)?;
            }
        }
    }
    Ok(())
}
