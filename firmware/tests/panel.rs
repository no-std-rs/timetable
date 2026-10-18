use core::convert::Infallible;
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use embedded_hal::{digital::OutputPin, spi::SpiBus};
use std::{cell::RefCell, rc::Rc};
use timetable_display::{App, Config, Screen, catalog::Catalog, config, render};
use timetable_firmware::panel::{Panel, TILE_BYTES, TILE_ROWS, Tile, WIDTH};

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
        for Pixel(p, color) in pixels {
            if (0..240).contains(&p.x) && (0..240).contains(&p.y) {
                self.0[p.y as usize * WIDTH + p.x as usize] = color;
            }
        }
        Ok(())
    }
}

#[test]
fn shared_settings_scratch_renders_every_screen_like_a_full_framebuffer() {
    let mut scratch = *include_bytes!("../../display/tests/settings.bin");
    let settings = config::decode(&mut scratch).unwrap().0;
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let mut app = App::default();
    for screen in [
        Screen::Board,
        Screen::Groups,
        Screen::Picks,
        Screen::Routes,
        Screen::Stops,
        Screen::Walk,
    ] {
        app.screen = screen;
        app.draft = Some(settings.groups[0].picks[0].clone());
        let mut frame = Frame([Rgb565::BLACK; WIDTH * 240]);
        render::draw(&mut frame, &app, &settings, &catalog).unwrap();
        for y in (0..240).step_by(TILE_ROWS) {
            // Saving settings can overwrite the entire buffer between frames.
            config::encode(&settings, 17, &mut scratch).unwrap();
            let pixels = scratch.first_chunk_mut::<TILE_BYTES>().unwrap();
            render::draw(&mut Tile { y, pixels }, &app, &settings, &catalog).unwrap();
            for (i, bytes) in pixels.chunks_exact(2).enumerate() {
                assert_eq!(
                    u16::from_be_bytes(bytes.try_into().unwrap()),
                    frame.0[y as usize * WIDTH + i].into_storage()
                );
            }
        }
        config::encode(&settings, 18, &mut scratch).unwrap();
        let (restored, generation): (Config, _) = config::decode(&mut scratch).unwrap();
        assert!(restored == settings);
        assert_eq!(generation, 18);
    }
}

#[test]
fn wire_pixels_clip_to_the_final_tile_and_keep_rgb565_byte_order() {
    let mut pixels = [0xab; TILE_BYTES];
    let mut tile = Tile {
        y: 224,
        pixels: &mut pixels,
    };
    tile.draw_iter([
        Pixel(Point::new(-1, 224), Rgb565::WHITE),
        Pixel(Point::new(0, 223), Rgb565::WHITE),
        Pixel(Point::new(0, 224), Rgb565::RED),
        Pixel(Point::new(239, 239), Rgb565::BLUE),
        Pixel(Point::new(240, 239), Rgb565::WHITE),
        Pixel(Point::new(1, 240), Rgb565::WHITE),
    ])
    .unwrap();
    tile.fill_solid(
        &Rectangle::new(Point::new(238, 238), Size::new(4, 4)),
        Rgb565::GREEN,
    )
    .unwrap();
    assert_eq!(&pixels[..2], &Rgb565::RED.into_storage().to_be_bytes());
    assert_eq!(&pixels[2..4], &[0xab; 2]);
    for row in [14, 15] {
        for column in [238, 239] {
            let i = (row * WIDTH + column) * 2;
            assert_eq!(
                &pixels[i..i + 2],
                &Rgb565::GREEN.into_storage().to_be_bytes()
            );
        }
    }
}

#[derive(Clone, Default)]
struct Spi(Rc<RefCell<Vec<Vec<u8>>>>);
impl embedded_hal::spi::ErrorType for Spi {
    type Error = Infallible;
}
impl SpiBus for Spi {
    fn read(&mut self, _: &mut [u8]) -> Result<(), Infallible> {
        unreachable!()
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), Infallible> {
        self.0.borrow_mut().push(bytes.to_vec());
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
fn panel_sends_the_complete_wire_tile_without_a_second_row_buffer() {
    let spi = Spi::default();
    let mut panel = Panel::new(spi.clone(), Pin);
    let mut pixels = [0; TILE_BYTES];
    for (i, byte) in pixels.iter_mut().enumerate() {
        *byte = i as u8;
    }
    panel.write_tile(224, &pixels).unwrap();
    let writes = spi.0.borrow();
    assert_eq!(
        &writes[..6],
        &[
            vec![0x2a],
            vec![0, 0, 0, 239],
            vec![0x2b],
            vec![0, 224, 0, 239],
            vec![0x2c],
            vec![]
        ]
    );
    assert_eq!(writes[6].as_slice(), pixels);
    assert_eq!(writes.len(), 7);
}
