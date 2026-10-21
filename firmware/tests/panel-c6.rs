#![cfg(feature = "board-waveshare-c6")]
use core::convert::Infallible;
use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use embedded_hal::{digital::OutputPin, spi::SpiBus};
use std::{cell::RefCell, rc::Rc};
use timetable_display::{App, Config, Screen, catalog::Catalog, render};
use timetable_firmware::panel::{HEIGHT, Panel, TILE_BYTES, TILE_ROWS, Tile, WIDTH};
#[derive(Clone, Default)]
struct Spi(Rc<RefCell<Vec<Vec<u8>>>>);
impl embedded_hal::spi::ErrorType for Spi {
    type Error = Infallible;
}
impl SpiBus for Spi {
    fn read(&mut self, _: &mut [u8]) -> Result<(), Infallible> {
        unreachable!()
    }
    fn write(&mut self, b: &[u8]) -> Result<(), Infallible> {
        self.0.borrow_mut().push(b.to_vec());
        Ok(())
    }
    fn transfer(&mut self, _: &mut [u8], _: &[u8]) -> Result<(), Infallible> {
        unreachable!()
    }
    fn transfer_in_place(&mut self, _: &mut [u8]) -> Result<(), Infallible> {
        unreachable!()
    }
    fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}
struct Pin;
impl embedded_hal::digital::ErrorType for Pin {
    type Error = Infallible;
}
impl OutputPin for Pin {
    fn set_low(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}
#[test]
fn visible_columns_and_last_tile_match_jd9853_reference() {
    let spi = Spi::default();
    let mut p = Panel::new(spi.clone(), Pin);
    let pixels = [0; TILE_BYTES];
    p.write_tile(304, &pixels).unwrap();
    let writes = spi.0.borrow();
    assert_eq!(writes[1], [0, 34, 0, 205]);
    assert_eq!(writes[3], [1, 48, 1, 63]);
    assert_eq!(writes[6].len(), 172 * 16 * 2);
}
struct Frame(Vec<Rgb565>);
impl OriginDimensions for Frame {
    fn size(&self) -> Size {
        Size::new(172, 320)
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
            if (0..172).contains(&p.x) && (0..320).contains(&p.y) {
                self.0[p.y as usize * 172 + p.x as usize] = c;
            }
        }
        Ok(())
    }
}
#[test]
fn every_portrait_screen_tiles_exactly_and_clips_right_edge() {
    let mut settings = *include_bytes!("../../display/tests/settings.bin");
    let config: Config = timetable_display::config::decode(&mut settings).unwrap().0;
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let mut app = App::default();
    app.draft = Some(config.groups[0].picks[0].clone());
    for screen in [
        Screen::Board,
        Screen::Groups,
        Screen::Picks,
        Screen::Routes,
        Screen::Stops,
        Screen::Walk,
        Screen::Freshness,
    ] {
        app.screen = screen;
        let mut frame = Frame(vec![Rgb565::BLACK; WIDTH * HEIGHT]);
        render::portrait::draw_animated(&mut frame, &app, &config, &catalog, 3).unwrap();
        for y in (0..HEIGHT as u16).step_by(TILE_ROWS) {
            let mut bytes = [0xab; TILE_BYTES];
            let mut tile = Tile {
                y,
                pixels: &mut bytes,
            };
            render::portrait::draw_animated(&mut tile, &app, &config, &catalog, 3).unwrap();
            tile.draw_iter([Pixel(Point::new(172, y.into()), Rgb565::RED)])
                .unwrap();
            for (i, b) in bytes.chunks_exact(2).enumerate() {
                assert_eq!(
                    u16::from_be_bytes(b.try_into().unwrap()),
                    frame.0[y as usize * WIDTH + i].into_storage(),
                    "{screen:?} tile {y} pixel {i}"
                );
            }
        }
    }
}
