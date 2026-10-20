//! Render the actual firmware UI to a PPM image on a development computer.
use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use std::{convert::Infallible, io::Write};
use timetable_display::{
    App, Config, Screen,
    catalog::Catalog,
    model::{Departure, Status},
    render,
};

struct Frame([Rgb565; 240 * 240]);
impl OriginDimensions for Frame {
    fn size(&self) -> Size {
        Size::new(240, 240)
    }
}
impl DrawTarget for Frame {
    type Color = Rgb565;
    type Error = Infallible;
    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Infallible> {
        for Pixel(p, c) in pixels {
            if (0..240).contains(&p.x) && (0..240).contains(&p.y) {
                self.0[p.y as usize * 240 + p.x as usize] = c;
            }
        }
        Ok(())
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut c: Config = serde_json_core::from_slice(include_bytes!("../tests/settings.json"))?.0;
    let catalog =
        Catalog::new(include_bytes!("../../catalog/catalog.bin")).ok_or("Invalid catalogue")?;
    let now = 1_792_301_400;
    let mut app = App {
        clock: Some(now),
        loaded_at: Some(now),
        freshness: timetable_display::freshness::Freshness {
            checked_at: Some(now - 120),
            source_updated_at: Some(now - 3 * 86400),
        },
        status: Status::Ready,
        ..App::default()
    };
    app.departures.reset(now);
    for minutes in [12, 21, 28, 35, 42, 49] {
        app.departures
            .push(Departure {
                at: now + minutes * 60,
                group: 0,
                pick: 0,
                conditional: false,
                headsign: 0,
            })
            .unwrap();
    }
    let mode = std::env::args().nth(2).unwrap_or_default();
    if matches!(mode.as_str(), "walk" | "routes" | "stops") {
        let pick = c.groups[0].picks[0].clone();
        app.selected_route = (0..catalog.route_count())
            .find(|&i| catalog.route(i).unwrap().id == pick.route)
            .unwrap_or(0);
        app.direction = pick.direction;
        app.draft = Some(pick);
        app.screen = match mode.as_str() {
            "walk" => Screen::Walk,
            "routes" => {
                app.cursor = app.selected_route / 4 * 4;
                Screen::Routes
            }
            _ => Screen::Stops,
        };
    } else if mode == "saved" {
        app.clock = None;
        app.status = Status::Connecting;
    } else if mode == "connecting" || mode == "restoring" || mode == "syncing" {
        app.clock = None;
        app.loaded_at = None;
        app.departures.clear();
        app.freshness = Default::default();
        app.status = match mode.as_str() {
            "restoring" => Status::Restoring,
            "syncing" => Status::Syncing,
            _ => Status::Connecting,
        };
    } else if mode == "setup" {
        app.group = usize::MAX;
    } else if mode == "empty" {
        c.groups[0].picks.clear();
        app.departures.clear();
    } else if mode == "offline" {
        app.status = Status::Offline;
    } else if mode == "loading" {
        app.status = Status::Loading;
    } else if mode == "urgent" {
        app.departures.truncate(1);
        app.clock = Some(now + 11 * 60);
    } else if mode == "freshness" {
        app.screen = Screen::Freshness;
    }
    // Optional elapsed seconds lets docs show the actual one-second countdown.
    if let Some(seconds) = std::env::args().nth(3) {
        let seconds: u32 = seconds.parse()?;
        app.clock = app.clock.map(|now| now + i64::from(seconds));
    }
    let mut frame = Frame([Rgb565::BLACK; 240 * 240]);
    let phase = std::env::args()
        .nth(4)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(app.clock.unwrap_or(0).rem_euclid(8) as u8);
    render::draw_animated(&mut frame, &app, &c, &catalog, phase)?;
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/timetable-preview.ppm".into());
    let mut out = std::fs::File::create(&path)?;
    out.write_all(b"P6\n240 240\n255\n")?;
    for p in frame.0 {
        out.write_all(&[
            (u16::from(p.r()) * 255 / 31) as u8,
            (u16::from(p.g()) * 255 / 63) as u8,
            (u16::from(p.b()) * 255 / 31) as u8,
        ])?;
    }
    println!("{path}");
    Ok(())
}
