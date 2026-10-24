//! Responsive tablet composition using the existing App, group scheduler and
//! walking advice. Tablets render text with native font shaping, not scaled pixels.
use crate::{Painter, Settings, State};
use core::fmt::Write;
use embedded_graphics::prelude::RgbColor;
use heapless::String;
use timetable_display::{
    App, Departure,
    model::{leave_in, walking_chance},
    render,
};

const PAPER: u32 = 0xfff5f1e8;
const CARD: u32 = 0xfffbf9f3;
const MUTED: u32 = 0xff646b60;
const LINE: u32 = 0xffdedbd0;
pub fn argb(color: embedded_graphics::pixelcolor::Rgb565) -> u32 {
    0xff00_0000
        | (u32::from(color.r()) * 255 / 31) << 16
        | (u32::from(color.g()) * 255 / 63) << 8
        | (u32::from(color.b()) * 255 / 31)
}
#[derive(Clone, Copy)]
pub struct Input {
    pub width: i32,
    pub height: i32,
    pub now: i64,
    pub monotonic_ms: i64,
    pub battery: i32,
    pub plugged: bool,
    pub motion_available: bool,
    /// 0 ready, 1 updating, 2 offline. Last validated data remains visible.
    pub status: u8,
}
struct Layout {
    w: i32,
    h: i32,
    margin: i32,
    gap: i32,
    tab_y: i32,
    tab_h: i32,
    body_y: i32,
    body_h: i32,
    wide: bool,
}
impl Layout {
    fn new(w: i32, h: i32) -> Self {
        let margin = (w / 24).clamp(16, 32);
        let gap = 16;
        Self {
            w,
            h,
            margin,
            gap,
            tab_y: 94,
            tab_h: 44,
            body_y: 158,
            body_h: (h - 158 - 64).max(0),
            wide: w >= 680 && h >= 420,
        }
    }
    fn tab_width(&self, count: usize) -> i32 {
        (self.w - 2 * self.margin - (count as i32 - 1) * 8) / count.max(1) as i32
    }
}
fn clock(settings: &Settings, at: i64) -> String<24> {
    let mut out = String::new();
    if let Some(time) = settings.timezone().local(at) {
        let _ = write!(out, "{:02}:{:02}", time.hour(), time.minute());
    } else {
        let _ = out.push_str("--:--");
    }
    out
}
fn journey_card<P: Painter>(
    settings: &Settings,
    app: &App,
    input: Input,
    l: &Layout,
    p: &mut P,
) -> Result<(), P::Error> {
    let x = l.margin;
    let y = l.body_y;
    let w = l.w - 2 * l.margin;
    let h = l.body_h;
    let ink = argb(render::INK);
    p.rect(x, y, w, h, 18, CARD)?;
    let Some(path) = app.journeys.best(app.group, input.now) else {
        let message = if input.status == 1 {
            "Calculating journeys on this tablet…"
        } else if input.status == 2 {
            "Journey search failed. Open Device info for details."
        } else if app.loaded_at.is_none() {
            "Waiting for timetable data."
        } else if input.now > app.journeys.until {
            "Refresh to calculate the next journey window."
        } else {
            "No reachable journey within the downloaded window."
        };
        text(p, message, x + 20, y + 48, 20, w - 40, ink, 1)?;
        return text(
            p,
            "Check walking times, destinations and transfer slack.",
            x + 20,
            y + 82,
            14,
            w - 40,
            MUTED,
            0,
        );
    };
    let cache = &app.journeys;
    let config = settings.groups[app.group].journey.as_ref().unwrap();
    let mut value = String::<160>::new();
    let _ = write!(
        value,
        "Arrive {} · {} min",
        clock(settings, cache.epoch + i64::from(path.arrival)),
        (cache.epoch + i64::from(path.arrival) - input.now + 59) / 60
    );
    text(p, &value, x + 20, y + 37, 28, w - 40, ink, 1)?;
    value.clear();
    let _ = write!(
        value,
        "Leave in {} min · {} min walk · {}",
        (cache.epoch + i64::from(path.leave) - input.now).max(0) / 60,
        path.walk,
        app.headsigns.get(path.legs[0].from).unwrap_or("?")
    );
    text(
        p,
        &value,
        x + 20,
        y + 66,
        16,
        w - 40,
        argb(render::CHILLI),
        4,
    )?;
    let step = ((h - 110) / i32::from(path.len)).clamp(31, 70);
    for (i, leg) in path.legs[..path.len as usize].iter().enumerate() {
        let top = y + 98 + i as i32 * step;
        value.clear();
        let _ = write!(
            value,
            "{}  {} → {}",
            app.headsigns.get(leg.line).unwrap_or("?"),
            clock(settings, cache.epoch + i64::from(leg.departure)),
            clock(settings, cache.epoch + i64::from(leg.arrival))
        );
        text(p, &value, x + 20, top, 20, w - 40, ink, 4)?;
        value.clear();
        if i != 0 {
            let _ = write!(
                value,
                "{} min transfer · ",
                (u32::from(leg.departure) - u32::from(path.legs[i - 1].arrival)) / 60
            );
        }
        let _ = write!(
            value,
            "{} → {}",
            app.headsigns.get(leg.from).unwrap_or("?"),
            app.headsigns.get(leg.to).unwrap_or("?")
        );
        text(p, &value, x + 20, top + 17, 12, w - 40, MUTED, 0)?;
    }
    value.clear();
    let _ = write!(
        value,
        "On-device search complete · up to {} transfers · {} min minimum change{}",
        config.max_transfers,
        config.transfer_minutes,
        if path.egress != 0 {
            " · includes destination walk"
        } else {
            ""
        }
    );
    text(p, &value, x + 20, y + h - 15, 11, w - 40, MUTED, 0)
}
#[allow(clippy::too_many_arguments)]
fn text<P: Painter>(
    p: &mut P,
    value: &str,
    x: i32,
    y: i32,
    size: i32,
    width: i32,
    color: u32,
    style: u32,
) -> Result<(), P::Error> {
    if width > 0 {
        p.text(value, x, y, size, width, color, style)?;
    }
    Ok(())
}
fn label(event: Departure, settings: &Settings) -> &timetable_display::Pick {
    &settings.groups[usize::from(event.group)].picks[usize::from(event.pick)]
}
#[allow(clippy::too_many_arguments)]
fn hero<P: Painter>(
    p: &mut P,
    app: &App,
    settings: &Settings,
    event: Option<Departure>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    now: i64,
) -> Result<(), P::Error> {
    let ink = argb(render::INK);
    let chilli = argb(render::CHILLI);
    p.rect(x, y, w, h, 22, argb(render::PORCELAIN))?;
    if let Some(event) = event {
        let pick = label(event, settings);
        let chance = walking_chance(event.at, now, pick);
        text(
            p,
            if event.conditional {
                "CONDITIONAL BOARDING"
            } else if chance >= 50 {
                "YOUR NEXT DEPARTURE"
            } else {
                "NEXT DEPARTURE"
            },
            x + 20,
            y + 28,
            11,
            w - 40,
            MUTED,
            4,
        )?;
        p.rect(x + 20, y + 44, 68.min(w - 40), 46, 12, chilli)?;
        text(p, &pick.line, x + 30, y + 75, 26, 50, CARD, 6)?;
        let destination = app
            .headsigns
            .get(event.headsign)
            .filter(|s| !s.is_empty())
            .unwrap_or(&pick.headsign);
        text(p, destination, x + 100, y + 65, 18, w - 120, ink, 4)?;
        text(p, &pick.stop_name, x + 100, y + 86, 12, w - 120, MUTED, 0)?;
        let mut due = String::<32>::new();
        let minutes = (event.at - now + 59).max(0) / 60;
        let _ = write!(due, "{minutes} min");
        let baseline = if h >= 240 { (h - 76).min(210) } else { h - 24 };
        let large = (w / 6).min(baseline - 106).clamp(28, 72);
        text(p, &due, x + 20, y + baseline, large, w - 40, ink, 1)?;
        if h >= 240 {
            let mut advice = String::<80>::new();
            let left = leave_in(event.at, now, pick);
            if chance == 0 {
                let _ = advice.push_str("Not enough walking time");
            } else if left <= 0 {
                let _ = advice.push_str("Leave now");
            } else {
                let _ = write!(advice, "Leave in {} min", (left + 59) / 60);
            }
            text(
                p,
                &advice,
                x + 20,
                y + h - 53,
                21,
                w - 40,
                if chance == 0 { chilli } else { ink },
                4,
            )?;
            let mut detail = String::<96>::new();
            let _ = write!(
                detail,
                "{}m walk · {}% walk est. · {}",
                pick.walk,
                chance,
                clock(settings, event.at)
            );
            text(p, &detail, x + 20, y + h - 25, 12, w - 40, MUTED, 0)?;
        }
    } else {
        text(
            p,
            if app.loaded_at.is_some() {
                "No upcoming departures"
            } else {
                "Your journeys, here."
            },
            x + 20,
            y + 48,
            26,
            w - 40,
            ink,
            1,
        )?;
        text(
            p,
            if settings.groups[app.group].picks.is_empty() {
                "Choose lines and stops on the website."
            } else if app.loaded_at.is_some() {
                "Refresh or try another group."
            } else {
                "Waiting for the first timetable download."
            },
            x + 20,
            y + 84,
            14,
            w - 40,
            MUTED,
            0,
        )?;
    }
    Ok(())
}
/// Returns power flags; the Android host only applies them to its window.
pub fn draw<P: Painter>(
    settings: &Settings,
    app: &mut App,
    state: &mut State,
    input: Input,
    p: &mut P,
) -> Result<u32, P::Error> {
    let l = Layout::new(input.width, input.height);
    let ink = argb(render::INK);
    let gold = argb(render::GOLD);
    p.rect(0, 0, l.w, l.h, 0, PAPER)?;
    if l.w < 280 || l.h < 420 {
        text(p, "Timetable", 16, 40, 25, l.w - 32, ink, 1)?;
        text(
            p,
            "Make this window a little larger.",
            16,
            72,
            14,
            l.w - 32,
            MUTED,
            0,
        )?;
        return Ok(u32::from(settings.keep_awake));
    }
    app.group = (state.group as usize).min(settings.groups.len() - 1);
    app.manual_until = state.manual_until;
    app.tick_in_zone(&settings.config(), input.now, settings.timezone());
    state.group = app.group as u32;
    text(p, "TIMETABLE", l.margin, 27, 12, l.w / 2, gold, 4)?;
    text(
        p,
        &settings.groups[app.group].name,
        l.margin,
        69,
        34,
        l.w - 2 * l.margin - 150,
        ink,
        1,
    )?;
    text(
        p,
        &clock(settings, input.now),
        l.w - l.margin - 138,
        64,
        36,
        138,
        ink,
        2,
    )?;
    text(p, "•••", l.w - l.margin - 28, 27, 16, 28, gold, 4)?;
    let tab_w = l.tab_width(settings.groups.len());
    for (i, group) in settings.groups.iter().enumerate() {
        let x = l.margin + i as i32 * (tab_w + 8);
        p.rect(
            x,
            l.tab_y,
            tab_w,
            l.tab_h,
            14,
            if i == app.group { gold } else { CARD },
        )?;
        text(
            p,
            &group.name,
            x + 12,
            l.tab_y + 28,
            14,
            tab_w - 24,
            if i == app.group { CARD } else { ink },
            4,
        )?;
    }
    if settings.groups[app.group].journey.is_some() {
        journey_card(settings, app, input, &l, p)?;
    } else {
        let featured = app.featured(&settings.config()).map(|(_, event)| event);
        let (hero_w, hero_h, list_x, list_y, list_w, list_h) = if l.wide {
            let hw = (l.w - 2 * l.margin - l.gap) * 53 / 100;
            (
                hw,
                l.body_h,
                l.margin + hw + l.gap,
                l.body_y,
                l.w - 2 * l.margin - hw - l.gap,
                l.body_h,
            )
        } else {
            let hh = (l.body_h * 55 / 100).clamp(240, 300).min(l.body_h);
            (
                l.w - 2 * l.margin,
                hh,
                l.margin,
                l.body_y + hh + l.gap,
                l.w - 2 * l.margin,
                (l.body_h - hh - l.gap).max(0),
            )
        };
        hero(
            p, app, settings, featured, l.margin, l.body_y, hero_w, hero_h, input.now,
        )?;
        let rows = (list_h / 74).max(0) as usize;
        for (i, event) in app
            .board_departures()
            .filter(|e| featured.is_none_or(|f| f.at != e.at || f.pick != e.pick))
            .take(rows)
            .enumerate()
        {
            let y = list_y + i as i32 * 74;
            let pick = label(event, settings);
            p.rect(list_x, y, list_w, 64, 14, CARD)?;
            let mut line = String::<12>::new();
            let _ = write!(
                line,
                "{}{}",
                pick.line,
                if event.conditional { "*" } else { "" }
            );
            text(
                p,
                &line,
                list_x + 14,
                y + 29,
                22,
                58,
                argb(render::CHILLI),
                6,
            )?;
            text(
                p,
                app.headsigns
                    .get(event.headsign)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&pick.headsign),
                list_x + 76,
                y + 25,
                15,
                list_w - 146,
                ink,
                4,
            )?;
            text(
                p,
                &pick.stop_name,
                list_x + 76,
                y + 46,
                11,
                list_w - 146,
                MUTED,
                0,
            )?;
            let mut due = String::<20>::new();
            let _ = write!(due, "{}m", (event.at - input.now + 59) / 60);
            text(p, &due, list_x + list_w - 62, y + 31, 23, 50, ink, 2)?;
        }
    }
    p.rect(l.margin, l.h - 48, l.w - 2 * l.margin, 1, 0, LINE)?;
    let mut footer = String::<128>::new();
    let _ = footer.push_str(match input.status {
        1 => "Updating · ",
        2 => "Offline · ",
        _ => "",
    });
    if let Some(at) = app.freshness.checked_at {
        if let Some(t) = settings.timezone().local(at) {
            let _ = write!(
                footer,
                "Checked {:02}/{:02} {:02}:{:02}",
                t.day(),
                t.month() as u8,
                t.hour(),
                t.minute()
            );
        }
    } else {
        let _ = footer.push_str("No saved timetable");
    }
    if input.battery >= 0 {
        let _ = write!(
            footer,
            " · {}%{}",
            input.battery,
            if input.plugged { " charging" } else { "" }
        );
    }
    text(
        p,
        &footer,
        l.margin,
        l.h - 23,
        11,
        l.w - 2 * l.margin - 62,
        MUTED,
        0,
    )?;
    text(p, "Refresh", l.w - l.margin - 54, l.h - 23, 11, 54, gold, 4)?;
    let dim = input.motion_available && state.dim(settings, input.monotonic_ms);
    Ok(u32::from(settings.keep_awake) | (u32::from(dim) << 1))
}
/// Touch hit testing uses exactly the same layout as drawing. Returns 1 for refresh, 2 for the actions menu, and 0 otherwise.
pub fn tap(
    settings: &Settings,
    state: &mut State,
    size: (i32, i32),
    point: (i32, i32),
    now: i64,
    monotonic_ms: i64,
) -> u8 {
    let (width, height) = size;
    let (x, y) = point;
    let l = Layout::new(width, height);
    state.active(monotonic_ms);
    if y < 40 && x >= l.w - l.margin - 48 {
        return 2;
    }
    if y >= l.tab_y && y < l.tab_y + l.tab_h {
        let tab_w = l.tab_width(settings.groups.len());
        for i in 0..settings.groups.len() {
            let left = l.margin + i as i32 * (tab_w + 8);
            if x >= left && x < left + tab_w {
                state.group = i as u32;
                state.manual_until = now.saturating_add(7200);
            }
        }
    } else if y < l.tab_y {
        state.manual_until = 0;
    }
    u8::from(y >= l.h - 48)
}
