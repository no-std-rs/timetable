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
    let c: Config = serde_json_core::from_slice(include_bytes!("../tests/settings.json"))?.0;
    let catalog =
        Catalog::new(include_bytes!("../../catalog/catalog.bin")).ok_or("Invalid catalogue")?;
    let now = 1_792_301_400;
    let mut app = App {
        clock: Some(now),
        loaded_at: Some(now),
        status: Status::Ready,
        ..App::default()
    };
    app.departures.reset(now);
    for minutes in [12, 21, 28] {
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
    } else if mode == "offline" {
        app.status = Status::Offline;
    } else if mode == "loading" {
        app.status = Status::Loading;
    } else if mode == "urgent" {
        app.departures.truncate(1);
        app.clock = Some(now + 11 * 60);
    }
    let mut frame = Frame([Rgb565::BLACK; 240 * 240]);
    render::draw(&mut frame, &app, &c, &catalog)?;
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
