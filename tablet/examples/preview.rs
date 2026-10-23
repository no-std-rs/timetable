//! Render the same Rust layout as Android into SVG for offline design review.
use std::{convert::Infallible, fmt::Write};
use timetable_display::{App, Departure};
use timetable_tablet::{Painter, Settings, State, board};
struct Svg {
    output: String,
    clip: usize,
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
impl Painter for Svg {
    type Error = Infallible;
    fn rect(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        radius: i32,
        color: u32,
    ) -> Result<(), Infallible> {
        writeln!(self.output,"<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"{radius}\" fill=\"#{:06x}\"/>",color&0xffffff).unwrap();
        Ok(())
    }
    fn text(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        size: i32,
        width: i32,
        color: u32,
        style: u32,
    ) -> Result<(), Infallible> {
        self.clip += 1;
        let id = self.clip;
        let family = match style & 3 {
            1 => "Georgia,serif",
            2 => "monospace",
            _ => "Arial,sans-serif",
        };
        writeln!(self.output,"<clipPath id=\"c{id}\"><rect x=\"{x}\" y=\"{}\" width=\"{width}\" height=\"{}\"/></clipPath><text x=\"{x}\" y=\"{y}\" font-family=\"{family}\" font-size=\"{size}\" font-weight=\"{}\" fill=\"#{:06x}\" clip-path=\"url(#c{id})\">{}</text>",y-size*2,size*3,if style&4!=0 {"bold"}else{"normal"},color&0xffffff,escape(text)).unwrap();
        Ok(())
    }
}
fn main() {
    let settings = Settings::decode(include_bytes!("../tests/fixtures/settings.json")).unwrap();
    let now = 1792216800;
    let mut app = App::default();
    app.departures.reset(now);
    app.loaded_at = Some(now);
    app.freshness.checked_at = Some(now - 60);
    let heading = app.headsigns.insert("City centre").unwrap();
    for n in 1..=20 {
        app.departures
            .push(Departure {
                at: now + n * 120,
                group: 0,
                pick: 0,
                conditional: false,
                headsign: heading,
            })
            .unwrap();
    }
    for (w, h) in [(1024, 600), (600, 1024), (360, 640)] {
        let mut svg = Svg {
            output: format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">"
            ),
            clip: 0,
        };
        board::draw(
            &settings,
            &mut app,
            &mut State::default(),
            board::Input {
                width: w,
                height: h,
                now,
                monotonic_ms: 1000,
                battery: 80,
                plugged: true,
                motion_available: false,
                status: 0,
            },
            &mut svg,
        )
        .unwrap();
        svg.output.push_str("</svg>");
        std::fs::write(format!("/tmp/timetable-tablet-{w}x{h}.svg"), svg.output).unwrap();
    }
}
