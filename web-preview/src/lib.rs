#![no_std]
//! Browser hardware adapter. UI, catalogue, settings and pixels are shared with firmware.

use core::convert::Infallible;
use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use timetable_display::{
    App, Config, Departure, Screen, catalog::Catalog, model::Status, render, ui,
};

pub const WIDTH: usize = 240;
pub const FRAME_BYTES: usize = WIDTH * WIDTH * 4;
pub const BUFFER_BYTES: usize = 8192;
pub const OUTPUT_BYTES: usize = 16384;
const CATALOG: &[u8] = include_bytes!("../../catalog/catalog.bin");

pub struct Frame(pub [u8; FRAME_BYTES]);
impl Default for Frame {
    fn default() -> Self {
        Self([0; FRAME_BYTES])
    }
}
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
    ) -> Result<(), Self::Error> {
        for Pixel(p, c) in pixels {
            if (0..240).contains(&p.x) && (0..240).contains(&p.y) {
                let offset = (p.y as usize * WIDTH + p.x as usize) * 4;
                self.0[offset..offset + 4].copy_from_slice(&[
                    (u16::from(c.r()) * 255 / 31) as u8,
                    (u16::from(c.g()) * 255 / 63) as u8,
                    (u16::from(c.b()) * 255 / 31) as u8,
                    255,
                ]);
            }
        }
        Ok(())
    }
}

pub struct Simulator {
    pub app: App,
    pub config: Config,
    catalog: Catalog<'static>,
    mode: u32,
}
impl Simulator {
    pub fn new(json: &[u8], now: i64, group: usize) -> Option<Self> {
        timetable_display::clock::warsaw(now)?;
        let mut unescape = [0; 64];
        let (config, used) =
            serde_json_core::from_slice_escaped::<Config>(json, &mut unescape).ok()?;
        if !json[used..].iter().all(u8::is_ascii_whitespace)
            || config.validate().is_err()
            || group >= config.groups.len()
        {
            return None;
        }
        let mut value = Self {
            app: App::default(),
            config,
            catalog: Catalog::new(CATALOG)?,
            mode: 0,
        };
        value.app.choose_group(group, &value.config);
        value.samples(now);
        value.app.tick(&value.config, now);
        Some(value)
    }

    fn samples(&mut self, now: i64) {
        self.app.departures.reset(now);
        self.app.headsigns.clear();
        for minutes in [12, 21, 28, 35, 42, 49] {
            for (group, g) in self.config.groups.iter().enumerate() {
                for (pick, p) in g.picks.iter().enumerate() {
                    let headsign = self.app.headsigns.insert(&p.headsign).unwrap();
                    self.app
                        .departures
                        .push(Departure {
                            at: now + minutes * 60 + pick as i64 * 60,
                            group: group as u8,
                            pick: pick as u8,
                            conditional: false,
                            headsign,
                        })
                        .unwrap();
                }
            }
        }
        self.app.departures.sort();
        self.app.loaded_at = Some(now);
        self.app.freshness.checked_at = Some(now - 120);
        self.app.freshness.source_updated_at = None; // Sample data has no actual source update.
    }

    /// Explicit browser hardware scenarios: ready, cached boot, Wi-Fi, NTP, update, offline.
    pub fn mode(&mut self, mode: u32, now: i64) -> bool {
        if mode > 5 || timetable_display::clock::warsaw(now).is_none() {
            return false;
        }
        self.mode = mode;
        self.app.status = match mode {
            1 | 2 => Status::Connecting,
            3 => Status::Syncing,
            4 => Status::Loading,
            5 => Status::Offline,
            _ => Status::Ready,
        };
        self.app.clock = if (1..=3).contains(&mode) {
            None
        } else {
            Some(now)
        };
        if mode == 2 {
            self.app.departures.clear();
            self.app.loaded_at = None;
            self.app.freshness = Default::default();
        } else if self.app.loaded_at.is_none() {
            self.samples(now);
        }
        true
    }

    pub fn tick(&mut self, now: i64) {
        if timetable_display::clock::warsaw(now).is_some() && self.app.clock.is_some() {
            self.app.tick(&self.config, now);
        }
    }
    pub fn draw(&self, frame: &mut Frame, phase: u8) {
        render::draw_animated(frame, &self.app, &self.config, &self.catalog, phase).unwrap();
    }
    pub fn gesture(&mut self, x: u16, y: u16, dx: i16, dy: i16, held: bool) -> ui::Action {
        if dx.unsigned_abs() > 35 || dy.unsigned_abs() > 35 {
            ui::swipe(&mut self.app, &self.config, &self.catalog, dx, dy);
            ui::Action::None
        } else if x < 240 && y < 240 {
            ui::tap(&mut self.app, &mut self.config, &self.catalog, x, y, held)
        } else {
            ui::Action::None
        }
    }
    pub fn complete_mock_refresh(&mut self, now: i64) {
        if timetable_display::clock::warsaw(now).is_none() {
            return;
        }
        self.samples(now);
        self.mode(0, now);
    }
    pub fn screen(&self) -> u32 {
        match self.app.screen {
            Screen::Board => 0,
            Screen::Groups => 1,
            Screen::Picks => 2,
            Screen::Routes => 3,
            Screen::Stops => 4,
            Screen::Walk => 5,
            Screen::Freshness => 6,
        }
    }
    pub fn json(&self, output: &mut [u8]) -> Option<usize> {
        #[derive(serde::Serialize)]
        struct Board<'a> {
            pick: &'a timetable_display::Pick,
            seconds: Option<i64>,
            left: Option<i64>,
            chance: Option<u8>,
        }
        let next = self.app.featured(&self.config).map(|(_, event)| {
            let pick = &self.config.groups[event.group as usize].picks[event.pick as usize];
            Board {
                pick,
                seconds: self.app.clock.map(|now| event.at - now),
                left: self
                    .app
                    .clock
                    .map(|now| timetable_display::model::leave_in(event.at, now, pick)),
                chance: self
                    .app
                    .clock
                    .map(|now| timetable_display::model::walking_chance(event.at, now, pick)),
            }
        });
        #[derive(serde::Serialize)]
        struct Snapshot<'a> {
            config: &'a Config,
            group: usize,
            screen: u32,
            clock_known: bool,
            manual: bool,
            loaded: bool,
            mode: u32,
            next: Option<Board<'a>>,
            draft: &'a Option<timetable_display::Pick>,
        }
        serde_json_core::to_slice(
            &Snapshot {
                config: &self.config,
                group: self.app.group,
                screen: self.screen(),
                clock_known: self.app.clock.is_some(),
                manual: self.app.manual_until > self.app.clock.unwrap_or(0)
                    || !self.config.automatic
                    || !self.config.groups.iter().any(|group| group.days != 0),
                loaded: self.app.loaded_at.is_some(),
                mode: self.mode,
                next,
                draft: &self.app.draft,
            },
            output,
        )
        .ok()
    }
}

#[cfg(target_arch = "wasm32")]
mod abi;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}
