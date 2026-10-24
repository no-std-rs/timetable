//! Kintsugi palette, with one 240×16 tile on the board. No framebuffer or heap.
use crate::{
    App, Config, Screen,
    catalog::Catalog,
    clock,
    model::{CacheUpdate, Status, leave_in, walking_chance},
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

fn journey_board<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    app: &App,
    config: &Config,
    catalog: &Catalog<'_>,
    width: i32,
    height: i32,
) -> Result<(), D::Error> {
    let profile = catalog.profile();
    let group = &config.groups[app.group];
    let center = width / 2;
    let chars = (width / 6 - 4) as usize;
    label_with_profile(
        profile,
        d,
        &short(&group.name, chars),
        center,
        16,
        INK,
        true,
    )?;
    let status = match app.status {
        Status::Capacity => "SEARCH LIMIT / TAP RETRY",
        Status::Failed => "SEARCH FAILED / RETRY",
        Status::Offline => "OFFLINE / TAP RETRY",
        Status::Loading | Status::Connecting | Status::Syncing => "CALCULATING ON DEVICE",
        _ => "SCHEDULED JOURNEY",
    };
    label_with_profile(profile, d, &short(status, chars), center, 32, MUTED, true)?;
    let Some(now) = app.clock else {
        return label_with_profile(profile, d, "Waiting for clock", center, 80, MUTED, true);
    };
    let Some(path) = app.journeys.best(app.group, now) else {
        let message = if app.loaded_at.is_none() {
            "Waiting for route data"
        } else if matches!(app.status, Status::Failed | Status::Capacity) {
            "Search incomplete"
        } else if now > app.journeys.until {
            "Refresh route data"
        } else {
            "No reachable journey"
        };
        label_with_profile(profile, d, &short(message, chars), center, 78, INK, true)?;
        return label_with_profile(
            profile,
            d,
            "Tap status to refresh",
            center,
            101,
            MUTED,
            true,
        );
    };
    let zone = profile.zone();
    let mut value = String::<96>::new();
    if let Some(arrival) = zone.local(app.journeys.epoch + i64::from(path.arrival)) {
        let _ = write!(
            value,
            "Arrive {:02}:{:02} / {}m",
            arrival.hour(),
            arrival.minute(),
            (app.journeys.epoch + i64::from(path.arrival) - now + 59) / 60
        );
    }
    label_with_profile(profile, d, &short(&value, chars), center, 52, INK, true)?;
    value.clear();
    let _ = write!(
        value,
        "Leave in {}m / {}m walk",
        (app.journeys.epoch + i64::from(path.leave) - now).max(0) / 60,
        path.walk
    );
    label_with_profile(profile, d, &short(&value, chars), center, 68, CHILLI, true)?;
    let step = if height < 200 { 22 } else { 40 };
    for (i, leg) in path.legs[..path.len as usize].iter().enumerate() {
        value.clear();
        if let (Some(departure), Some(arrival)) = (
            zone.local(app.journeys.epoch + i64::from(leg.departure)),
            zone.local(app.journeys.epoch + i64::from(leg.arrival)),
        ) {
            let _ = write!(
                value,
                "{} {:02}:{:02}>{:02}:{:02}",
                app.headsigns.get(leg.line).unwrap_or("?"),
                departure.hour(),
                departure.minute(),
                arrival.hour(),
                arrival.minute()
            );
        }
        label_with_profile(
            profile,
            d,
            &short(&value, chars),
            center,
            88 + i as i32 * step,
            INK,
            true,
        )?;
        value.clear();
        if i != 0 {
            let wait = (u32::from(leg.departure) - u32::from(path.legs[i - 1].arrival)) / 60;
            let _ = write!(value, "{wait}m change / ");
        }
        let _ = write!(
            value,
            "{} > {}",
            app.headsigns.get(leg.from).unwrap_or("?"),
            app.headsigns.get(leg.to).unwrap_or("?")
        );
        label_with_profile(
            profile,
            d,
            &short(&value, chars),
            center,
            100 + i as i32 * step,
            MUTED,
            true,
        )?;
    }
    value.clear();
    let _ = write!(value, "{} transfer(s) / search complete", path.len - 1);
    label_with_profile(
        profile,
        d,
        &short(&value, chars),
        center,
        height - 16,
        GOLD,
        true,
    )
}

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
    label_with_profile(
        crate::profile::Profile::DEFAULT,
        d,
        text,
        x,
        y,
        color,
        center,
    )
}
fn label_with_profile<D: DrawTarget<Color = Rgb565>>(
    profile: crate::profile::Profile<'_>,
    d: &mut D,
    text: &str,
    x: i32,
    y: i32,
    color: Rgb565,
    center: bool,
) -> Result<(), D::Error> {
    let bounds = d.bounding_box();
    if y + 10 <= bounds.top_left.y || y >= bounds.bottom_right().map_or(0, |p| p.y + 1) {
        return Ok(());
    }
    let unsupported = |c: char| {
        !c.is_ascii()
            && c != '?'
            && FONT_6X10.glyph_mapping.index(c) == FONT_6X10.glyph_mapping.index('?')
    };
    if profile.has_glyphs() && text.chars().any(unsupported) {
        let width = |c| {
            if unsupported(c) && profile.glyph(c).is_some() {
                10
            } else {
                6
            }
        };
        let limit = if center {
            2 * (x - 12).min(228 - x).max(0)
        } else {
            (232 - x).max(0)
        };
        let total: i32 = text.chars().map(width).sum();
        let mut visible = String::<128>::new();
        let mut used = 0;
        for c in text.chars() {
            if total > limit && used + width(c) > limit - 12 {
                let _ = visible.push_str("..");
                used += 12;
                break;
            }
            if visible.push(c).is_err() {
                break;
            }
            used += width(c);
        }
        let mut left = if center { x - used / 2 } else { x };
        for c in visible.chars() {
            if unsupported(c)
                && let Some(bitmap) = profile.glyph(c)
            {
                if y < d.bounding_box().bottom_right().map_or(0, |p| p.y + 1)
                    && y + 10 > d.bounding_box().top_left.y
                {
                    d.draw_iter((0..10).flat_map(|row| {
                        let bits = u16::from_le_bytes([bitmap[row * 2], bitmap[row * 2 + 1]]);
                        (0..10)
                            .filter(move |col| bits & (1 << col) != 0)
                            .map(move |col| Pixel(Point::new(left + col, y + row as i32), color))
                    }))?;
                }
                left += 10;
            } else {
                let mut buf = [0; 4];
                draw_visible(
                    d,
                    &Text::with_baseline(
                        c.encode_utf8(&mut buf),
                        Point::new(left, y),
                        MonoTextStyle::new(&FONT_6X10, color),
                        Baseline::Top,
                    ),
                )?;
                left += 6;
            }
        }
        return Ok(());
    }
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

/// Old checks show their date rather than a deceptively recent-looking time.
pub fn freshness_line(app: &App) -> String<40> {
    freshness_in_zone(app, clock::Zone::WARSAW)
}
fn freshness_in_zone(app: &App, zone: clock::Zone<'_>) -> String<40> {
    let mut text = String::new();
    if let Some(checked) = app.freshness.checked_at.and_then(|at| zone.local(at)) {
        if app
            .clock
            .and_then(|at| zone.local(at))
            .is_some_and(|now| now.date() == checked.date())
        {
            let _ = write!(text, "Chk{:02}:{:02}", checked.hour(), checked.minute());
        } else {
            let _ = write!(text, "Chk{:02}/{:02}", checked.day(), checked.month() as u8);
        }
    } else {
        let _ = text.push_str("Chk --:--");
    }
    if let Some(source) = app
        .freshness
        .source_updated_at
        .and_then(|at| zone.local(at))
    {
        let _ = write!(text, " Src{:02}/{:02}", source.day(), source.month() as u8);
    } else {
        let _ = text.push_str(" Src --/--");
    }
    text
}

fn countdown(seconds: i64) -> String<12> {
    let seconds = seconds.max(0);
    let mut text = String::new();
    let _ = write!(text, "{}:{:02}", seconds / 60, seconds % 60);
    text
}

fn departure_time(at: i64, zone: clock::Zone<'_>) -> String<12> {
    let mut text = String::new();
    if let Some(local) = zone.local(at) {
        let _ = write!(text, "{:02}:{:02}", local.hour(), local.minute());
    } else {
        let _ = text.push_str("--:--");
    }
    text
}

fn timestamp(stamp: Option<i64>, unknown: &str, zone: clock::Zone<'_>) -> String<40> {
    let mut text = String::new();
    if let Some(local) = stamp.and_then(|at| zone.local(at)) {
        let _ = write!(
            text,
            "{:02}/{:02}/{:04} {:02}:{:02}",
            local.day(),
            local.month() as u8,
            local.year(),
            local.hour(),
            local.minute()
        );
    } else {
        let _ = text.push_str(unknown);
    }
    text
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
    label(d, "timetable", 120, 34, INK, true)?;
    label(d, "Your stops, one glance.", 120, 57, MUTED, true)?;
    label(d, "1  Open the installer", 120, 82, INK, true)?;
    label(d, "no-std-rs.github.io", 120, 99, INK, true)?;
    label(d, "/timetable", 120, 111, CHILLI, true)?;
    label(d, "2  Choose stops + walk time", 120, 143, INK, true)?;
    label(d, "3  Connect USB and install", 120, 163, INK, true)?;
    pill(d, 51, 184, 138, 23, GOLD)?;
    label(d, "Chrome / Edge", 120, 191, INK, true)?;
    label(d, "USB data cable", 120, 216, MUTED, true)
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
    draw_animated(
        d,
        app,
        config,
        catalog,
        app.clock.unwrap_or(0).rem_euclid(8) as u8,
    )
}

// All native layouts share the same decision and finger-sized button regions.
fn cache_confirmation<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    profile: crate::profile::Profile<'_>,
    width: i32,
) -> Result<(), D::Error> {
    let (title, top, height, margin) = match width {
        172 => (72, 230, 48, 8),
        320 => (18, 116, 44, 12),
        _ => (40, 156, 36, 28),
    };
    let center = width / 2;
    for (value, y, color) in [
        ("Save update?", title, CHILLI),
        ("Cache saved <24h ago.", title + 30, INK),
        ("More writes wear flash.", title + 46, MUTED),
        ("Update saved data?", title + 70, INK),
    ] {
        label_with_profile(profile, d, value, center, y, color, true)?;
    }
    let button_width = center - margin - 4;
    pill(d, margin, top, button_width as u32, height, GOLD)?;
    pill(d, center + 4, top, button_width as u32, height, CHILLI)?;
    label_with_profile(
        profile,
        d,
        "Keep",
        margin + button_width / 2,
        top + height as i32 / 2 - 5,
        INK,
        true,
    )?;
    label_with_profile(
        profile,
        d,
        "Update",
        center + 4 + button_width / 2,
        top + height as i32 / 2 - 5,
        PORCELAIN,
        true,
    )
}

/// The UI task supplies a monotonic phase so connection/NTP activity animates
/// even before UTC is known. Cached countdowns continue to use synchronized UTC.
pub fn draw_animated<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    app: &App,
    config: &Config,
    catalog: &Catalog<'_>,
    phase: u8,
) -> Result<(), D::Error> {
    let profile = catalog.profile();
    background(d)?;
    if app.cache_update == CacheUpdate::Pending {
        return cache_confirmation(d, profile, 240);
    }
    let Some(group) = config.groups.get(app.group) else {
        return setup(d);
    };
    if app.screen == Screen::Board && group.journey.is_some() {
        return journey_board(d, app, config, catalog, 240, 240);
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
    label_with_profile(profile, d, &short(title, 20), 120, 24, INK, true)?;
    if app.screen != Screen::Board {
        label_with_profile(profile, d, "<", 45, 25, INK, false)?;
    }
    match app.screen {
        Screen::Board => {
            let now = app.clock.unwrap_or(0);
            let chosen = app.featured(config);
            if let Some((chosen_index, event)) = chosen {
                let p = &group.picks[event.pick as usize];
                pill(d, 32, 56, 47, 29, INK)?;
                if p.line.chars().any(|c| {
                    !c.is_ascii()
                        && c != '?'
                        && FONT_6X10.glyph_mapping.index(c) == FONT_6X10.glyph_mapping.index('?')
                }) {
                    label_with_profile(profile, d, &p.line, 55, 65, PORCELAIN, true)?;
                } else {
                    draw_visible(
                        d,
                        &Text::with_text_style(
                            p.line.as_str(),
                            Point::new(55, 60),
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
                }
                let value = if app.clock.is_some() {
                    countdown(event.at - now)
                } else {
                    departure_time(event.at, catalog.profile().zone())
                };
                big(d, &value, 216 - value.len() as i32 * 20, 51)?;
                label_with_profile(
                    profile,
                    d,
                    if app.clock.is_some() { "IN" } else { "AT" },
                    84,
                    70,
                    MUTED,
                    false,
                )?;
                label_with_profile(
                    profile,
                    d,
                    &short(
                        app.headsigns
                            .get(event.headsign)
                            .filter(|s| !s.is_empty())
                            .unwrap_or(&p.headsign),
                        27,
                    ),
                    120,
                    92,
                    INK,
                    true,
                )?;
                let mut platform =
                    short(&p.stop_name, if p.stop_code.is_empty() { 27 } else { 20 });
                if !p.stop_code.is_empty() {
                    let _ = write!(platform, " / {}", p.stop_code);
                }
                label_with_profile(profile, d, &short(&platform, 31), 120, 104, MUTED, true)?;
                let mut text = String::<64>::new();
                let left = leave_in(event.at, now, p);
                let urgent = app.clock.is_some() && left < 60;
                if app.clock.is_none() {
                    if let Some(saved) = app
                        .loaded_at
                        .and_then(|at| catalog.profile().zone().local(at))
                    {
                        let _ = write!(
                            text,
                            "Saved {:02}/{:02} / clock pending",
                            saved.day(),
                            saved.month() as u8
                        );
                    } else {
                        let _ = text.push_str("Saved times / clock pending");
                    }
                } else if walking_chance(event.at, now, p) == 0 {
                    let _ = text.push_str("Not enough walk time");
                } else if left > 0 {
                    let _ = write!(text, "Leave in {}", countdown(left));
                } else {
                    let _ = text.push_str("Leave now");
                }
                pill(d, 29, 118, 182, 21, if urgent { CHILLI } else { GOLD })?;
                label_with_profile(
                    profile,
                    d,
                    &text,
                    120,
                    124,
                    if urgent { PORCELAIN } else { INK },
                    true,
                )?;
                text.clear();
                if app.clock.is_some() {
                    let _ = write!(
                        text,
                        "{}m walk / {}% walk est.{}",
                        p.walk,
                        walking_chance(event.at, now, p),
                        if event.conditional { " *" } else { "" }
                    );
                } else {
                    let _ = text.push_str("Countdowns after clock sync");
                }
                label_with_profile(profile, d, &text, 120, 141, MUTED, true)?;
                for (i, next) in app
                    .board_departures()
                    .enumerate()
                    .filter(|(index, _)| *index != chosen_index)
                    .map(|(_, event)| event)
                    .take(5)
                    .enumerate()
                {
                    let Some(p) = group.picks.get(next.pick as usize) else {
                        continue;
                    };
                    let y = 154 + i as i32 * 12;
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
                    label_with_profile(
                        profile,
                        d,
                        &short(&text, 20 - i.saturating_sub(2) * 2),
                        35 + i.saturating_sub(2) as i32 * 6,
                        y,
                        if app.clock.is_some() && walking_chance(next.at, now, p) < 50 {
                            CHILLI
                        } else {
                            INK
                        },
                        false,
                    )?;
                    text.clear();
                    if app.clock.is_some() {
                        let _ = write!(text, "{}m", (next.at - now + 59) / 60);
                    } else {
                        let _ = text.push_str(&departure_time(next.at, catalog.profile().zone()));
                    }
                    label_with_profile(
                        profile,
                        d,
                        &text,
                        (211 - i.saturating_sub(2) as i32 * 8) - text.len() as i32 * 6,
                        y,
                        INK,
                        false,
                    )?;
                }
            } else {
                let status = match app.status {
                    Status::Restoring => "Opening saved timetable",
                    Status::Connecting => "Connecting to Wi-Fi",
                    Status::Syncing => "Setting the clock",
                    Status::Loading => "Reading the timetable",
                    Status::Offline => "Offline / tap status to retry",
                    Status::Capacity => "Selection too large / hold to edit",
                    Status::Failed => "Update failed / tap status",
                    _ if group.picks.is_empty() => "Hold to add lines + stops",
                    _ => "No departures in next 6h",
                };
                label_with_profile(
                    profile,
                    d,
                    if group.picks.is_empty() {
                        "Your next journey"
                    } else if app.loaded_at.is_none() {
                        "Your stops are saved"
                    } else {
                        "Your timetable"
                    },
                    120,
                    65,
                    INK,
                    true,
                )?;
                label_with_profile(profile, d, status, 120, 84, MUTED, true)?;
                if group.picks.is_empty() {
                    label_with_profile(
                        profile,
                        d,
                        "Hold the screen to choose",
                        120,
                        118,
                        INK,
                        true,
                    )?;
                    label_with_profile(
                        profile,
                        d,
                        "a line, stop and walking time",
                        120,
                        133,
                        MUTED,
                        true,
                    )?;
                    label_with_profile(profile, d, "Swipe to change group", 120, 170, MUTED, true)?;
                } else {
                    for (i, pick) in group.picks.iter().take(4).enumerate() {
                        let y = 109 + i as i32 * 24;
                        pill(d, 29, y + 1, 30, 18, INK)?;
                        label_with_profile(
                            profile,
                            d,
                            &short(&pick.line, 4),
                            44,
                            y + 6,
                            PORCELAIN,
                            true,
                        )?;
                        label_with_profile(
                            profile,
                            d,
                            &short(&pick.stop_name, 23),
                            66,
                            y,
                            INK,
                            false,
                        )?;
                        label_with_profile(
                            profile,
                            d,
                            &short(&pick.headsign, 23),
                            66,
                            y + 11,
                            MUTED,
                            false,
                        )?;
                    }
                    if group.picks.len() <= 2
                        && app.loaded_at.is_none()
                        && matches!(
                            app.status,
                            Status::Connecting | Status::Syncing | Status::Loading
                        )
                    {
                        let y = 124 + group.picks.len() as i32 * 24;
                        label_with_profile(
                            profile,
                            d,
                            "First download takes longer",
                            120,
                            y,
                            MUTED,
                            true,
                        )?;
                        label_with_profile(
                            profile,
                            d,
                            "Later boots use saved times",
                            120,
                            y + 14,
                            MUTED,
                            true,
                        )?;
                    }
                }
            }
            let mut foot = String::<40>::new();
            let fault = app.can_refresh(config) || app.status == Status::Capacity;
            let saved_without_clock = app.clock.is_none() && app.loaded_at.is_some();
            if !fault
                && let Some(local) = catalog
                    .profile()
                    .zone()
                    .local(now)
                    .filter(|_| app.clock.is_some())
            {
                let _ = write!(
                    foot,
                    "{:02}:{:02}:{:02} / ",
                    local.hour(),
                    local.minute(),
                    local.second()
                );
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
            } else if saved_without_clock {
                "SAVED / CLOCK PENDING"
            } else if app.status == Status::Restoring {
                "OPENING SAVED DATA"
            } else if app.status == Status::Connecting {
                "CONNECTING"
            } else if app.status == Status::Syncing {
                "SYNCING"
            } else if app.status == Status::Loading {
                match now.rem_euclid(3) {
                    0 => "UPDATING.",
                    1 => "UPDATING..",
                    _ => "UPDATING...",
                }
            } else if now < app.manual_until
                || !config.automatic
                || !config.groups.iter().any(|g| g.days != 0)
            {
                "MANUAL"
            } else {
                "SCHEDULED"
            });
            label_with_profile(
                profile,
                d,
                &foot,
                if saved_without_clock { 126 } else { 120 },
                42,
                if app.save_failed || app.sleep_failed || fault {
                    CHILLI
                } else {
                    MUTED
                },
                true,
            )?;
            if matches!(
                app.status,
                Status::Restoring | Status::Connecting | Status::Syncing | Status::Loading
            ) {
                for (i, (x, y)) in [
                    (0, -4),
                    (3, -3),
                    (4, 0),
                    (3, 3),
                    (0, 4),
                    (-3, 3),
                    (-4, 0),
                    (-3, -3),
                ]
                .into_iter()
                .enumerate()
                {
                    draw_visible(
                        d,
                        &Circle::new(Point::new(48 + x, 47 + y), 2).into_styled(
                            PrimitiveStyle::with_fill(if i == usize::from(phase % 8) {
                                CHILLI
                            } else {
                                GOLD
                            }),
                        ),
                    )?;
                }
            } else if !fault {
                draw_visible(
                    d,
                    &Circle::new(Point::new(47, 44), 4).into_styled(PrimitiveStyle::with_fill(
                        if app.status == Status::Ready {
                            GOLD
                        } else {
                            CHILLI
                        },
                    )),
                )?;
            }
            label_with_profile(
                profile,
                d,
                &freshness_in_zone(app, catalog.profile().zone()),
                120,
                212,
                MUTED,
                true,
            )?;
        }
        Screen::Freshness => {
            label_with_profile(profile, d, "Last successful check", 120, 62, MUTED, true)?;
            label_with_profile(
                profile,
                d,
                &timestamp(
                    app.freshness.checked_at,
                    "Not checked yet",
                    catalog.profile().zone(),
                ),
                120,
                80,
                INK,
                true,
            )?;
            label_with_profile(profile, d, "Source last updated", 120, 119, MUTED, true)?;
            label_with_profile(
                profile,
                d,
                &timestamp(
                    app.freshness.source_updated_at,
                    "Not supplied",
                    catalog.profile().zone(),
                ),
                120,
                137,
                INK,
                true,
            )?;
            label_with_profile(profile, d, "Oldest source file used", 120, 155, MUTED, true)?;
            label_with_profile(
                profile,
                d,
                catalog.profile().timezone().unwrap_or("Local time"),
                120,
                178,
                MUTED,
                true,
            )?;
            label_with_profile(
                profile,
                d,
                if app.can_refresh(config) {
                    "Retry now"
                } else {
                    "Tap to return"
                },
                120,
                207,
                if app.can_refresh(config) {
                    CHILLI
                } else {
                    MUTED
                },
                true,
            )?;
        }
        Screen::Groups => {
            for (i, g) in config.groups.iter().enumerate() {
                if i == app.group {
                    pill(d, 30, 50 + i as i32 * 36, 180, 31, GOLD)?;
                }
                label_with_profile(profile, d, &g.name, 120, 61 + i as i32 * 36, INK, true)?;
            }
            label_with_profile(
                profile,
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
                label_with_profile(
                    profile,
                    d,
                    &short(&row, 27),
                    32,
                    63 + i as i32 * 32,
                    INK,
                    false,
                )?;
            }
            if group.picks.len() < crate::config::MAX_PICKS {
                label_with_profile(
                    profile,
                    d,
                    "+ Add a line + stop",
                    32,
                    63 + group.picks.len() as i32 * 32,
                    CHILLI,
                    false,
                )?;
            }
            label_with_profile(
                profile,
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
                    label_with_profile(
                        profile,
                        d,
                        &short(&row, 27),
                        31,
                        66 + i as i32 * 32,
                        INK,
                        false,
                    )?;
                }
            }
            page_count(d, app.cursor, catalog.route_count())?;
            label_with_profile(profile, d, "< previous      next >", 120, 204, MUTED, true)?;
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
                label_with_profile(
                    profile,
                    d,
                    &short(s.name, 27),
                    31,
                    61 + i as i32 * 32,
                    INK,
                    false,
                )?;
                label_with_profile(profile, d, s.code, 31, 73 + i as i32 * 32, MUTED, false)?;
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
            label_with_profile(profile, d, "<", 52, 204, MUTED, true)?;
            label_with_profile(profile, d, &foot, 120, 204, INK, true)?;
            label_with_profile(profile, d, ">", 188, 204, MUTED, true)?;
        }
        Screen::Walk => {
            if let Some(p) = &app.draft {
                let mut row = String::<96>::new();
                pill(d, 35, 43, 39, 20, INK)?;
                label_with_profile(profile, d, &short(&p.line, 4), 55, 48, PORCELAIN, true)?;
                label_with_profile(profile, d, &short(&p.stop_name, 17), 83, 48, INK, false)?;
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
                    label_with_profile(profile, d, &row, 120, 78 + i as i32 * 38, INK, true)?;
                }
                label_with_profile(
                    profile,
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
                label_with_profile(profile, d, "Save", 153, 200, INK, true)?;
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

#[cfg(test)]
mod countdown_tests {
    #[test]
    fn seconds_are_exact_at_minute_and_horizon_boundaries() {
        for (seconds, text) in [
            (-1, "0:00"),
            (0, "0:00"),
            (1, "0:01"),
            (59, "0:59"),
            (60, "1:00"),
            (599, "9:59"),
            (600, "10:00"),
            (21600, "360:00"),
        ] {
            assert_eq!(super::countdown(seconds).as_str(), text);
        }
    }
}

pub mod portrait;

pub mod landscape;
