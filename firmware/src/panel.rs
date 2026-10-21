//! Bounded wire tiles for GUITION GC9A01 and Waveshare C6 Touch JD9853.
//! GC9A01 values follow pawelchcki/razem-tag; JD9853 follows Waveshare.

use core::convert::Infallible;
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use embedded_hal::{delay::DelayNs, digital::OutputPin, spi::SpiBus};

pub const WIDTH: usize = if cfg!(feature = "c6-landscape") {
    320
} else if cfg!(feature = "board-waveshare-c6") {
    172
} else {
    240
};
pub const HEIGHT: usize = if cfg!(feature = "c6-landscape") {
    172
} else if cfg!(feature = "board-waveshare-c6") {
    320
} else {
    240
};
pub const TILE_ROWS: usize = if cfg!(feature = "c6-landscape") {
    8
} else {
    16
};
pub const TILE_BYTES: usize = WIDTH * TILE_ROWS * 2;

pub struct Panel<SPI, DC> {
    spi: SPI,
    dc: DC,
}

impl<SPI: SpiBus, DC: OutputPin<Error = Infallible>> Panel<SPI, DC> {
    pub fn new(spi: SPI, dc: DC) -> Self {
        Self { spi, dc }
    }

    fn command(&mut self, command: u8, data: &[u8]) -> Result<(), SPI::Error> {
        self.dc.set_low().unwrap();
        self.spi.write(&[command])?;
        self.spi.flush()?;
        self.dc.set_high().unwrap();
        self.spi.write(data)?;
        self.spi.flush()
    }

    #[cfg(not(feature = "board-waveshare-c6"))]
    pub fn init(&mut self, delay: &mut impl DelayNs) -> Result<(), SPI::Error> {
        self.command(0x01, &[])?; // Software reset: no LCD reset GPIO on this board.
        delay.delay_ms(150);
        for (cmd, data) in [
            (0xEF, &[][..]),
            (0xEB, &[0x14]),
            (0xFE, &[]),
            (0xEF, &[]),
            (0xEB, &[0x14]),
            (0x84, &[0x40]),
            (0x85, &[0xFF]),
            (0x86, &[0xFF]),
            (0x87, &[0xFF]),
            (0x88, &[0x0A]),
            (0x89, &[0x21]),
            (0x8A, &[0x00]),
            (0x8B, &[0x80]),
            (0x8C, &[0x01]),
            (0x8D, &[0x01]),
            (0x8E, &[0xFF]),
            (0x8F, &[0xFF]),
            (0xB6, &[0x00, 0x20]),
            (0x36, &[0x08]),
            (0x3A, &[0x55]),
            (0x90, &[0x08, 0x08, 0x08, 0x08]),
            (0xBD, &[0x06]),
            (0xBC, &[0x00]),
            (0xFF, &[0x60, 0x01, 0x04]),
            (0xC3, &[0x13]),
            (0xC4, &[0x13]),
            (0xC9, &[0x22]),
            (0xBE, &[0x11]),
            (0xE1, &[0x10, 0x0E]),
            (0xDF, &[0x20, 0x0C, 0x02]),
            (0xF0, &[0x45, 0x09, 0x08, 0x08, 0x26, 0x2A]),
            (0xF1, &[0x43, 0x70, 0x72, 0x36, 0x37, 0x6F]),
            (0xF2, &[0x45, 0x09, 0x08, 0x08, 0x26, 0x2A]),
            (0xF3, &[0x43, 0x70, 0x72, 0x36, 0x37, 0x6F]),
            (0xED, &[0x18, 0x0B]),
            (0xAE, &[0x77]),
            (0xCD, &[0x63]),
            (
                0x70,
                &[0x07, 0x07, 0x04, 0x0E, 0x0F, 0x09, 0x07, 0x08, 0x03],
            ),
            (0xE8, &[0x34]),
            (
                0x62,
                &[
                    0x18, 0x0D, 0x71, 0xED, 0x70, 0x70, 0x18, 0x0F, 0x71, 0xEF, 0x70, 0x70,
                ],
            ),
            (
                0x63,
                &[
                    0x18, 0x11, 0x71, 0xF1, 0x70, 0x70, 0x18, 0x13, 0x71, 0xF3, 0x70, 0x70,
                ],
            ),
            (0x64, &[0x28, 0x29, 0xF1, 0x01, 0xF1, 0x00, 0x07]),
            (
                0x66,
                &[0x3C, 0x00, 0xCD, 0x67, 0x45, 0x45, 0x10, 0x00, 0x00, 0x00],
            ),
            (
                0x67,
                &[0x00, 0x3C, 0x00, 0x00, 0x00, 0x01, 0x54, 0x10, 0x32, 0x98],
            ),
            (0x74, &[0x10, 0x85, 0x80, 0x00, 0x00, 0x4E, 0x00]),
            (0x98, &[0x3E, 0x07]),
            (0x21, &[]),
            (0x11, &[]),
        ] {
            self.command(cmd, data)?;
        }
        delay.delay_ms(120);
        self.command(0x29, &[])
    }

    /// JD9853 register values from Waveshare's ESP32-C6-Touch-LCD-1.47 demo.
    /// Portrait RGB565, inversion enabled, visible columns 34..205.
    #[cfg(feature = "board-waveshare-c6")]
    pub fn init(&mut self, delay: &mut impl DelayNs) -> Result<(), SPI::Error> {
        self.command(0x01, &[])?;
        delay.delay_ms(150);
        self.command(
            0x36,
            &[if cfg!(feature = "c6-landscape") {
                0x60
            } else {
                0x00
            }],
        )?;
        for (cmd, data, ms) in [
            (0x11, &[][..], 120),
            (0xDF, &[0x98, 0x53][..], 0),
            (0xDF, &[0x98, 0x53][..], 0),
            (0xB2, &[0x23][..], 0),
            (0xB7, &[0x00, 0x47, 0x00, 0x6F][..], 0),
            (0xBB, &[0x1C, 0x1A, 0x55, 0x73, 0x63, 0xF0][..], 0),
            (0xC0, &[0x44, 0xA4][..], 0),
            (0xC1, &[0x16][..], 0),
            (
                0xC3,
                &[0x7D, 0x07, 0x14, 0x06, 0xCF, 0x71, 0x72, 0x77][..],
                0,
            ),
            (
                0xC4,
                &[
                    0x00, 0x00, 0xA0, 0x79, 0x0B, 0x0A, 0x16, 0x79, 0x0B, 0x0A, 0x16, 0x82,
                ][..],
                0,
            ),
            (
                0xC8,
                &[
                    0x3F, 0x32, 0x29, 0x29, 0x27, 0x2B, 0x27, 0x28, 0x28, 0x26, 0x25, 0x17, 0x12,
                    0x0D, 0x04, 0x00, 0x3F, 0x32, 0x29, 0x29, 0x27, 0x2B, 0x27, 0x28, 0x28, 0x26,
                    0x25, 0x17, 0x12, 0x0D, 0x04, 0x00,
                ][..],
                0,
            ),
            (0xD0, &[0x04, 0x06, 0x6B, 0x0F, 0x00][..], 0),
            (0xD7, &[0x00, 0x30][..], 0),
            (0xE6, &[0x14][..], 0),
            (0xDE, &[0x01][..], 0),
            (0xB7, &[0x03, 0x13, 0xEF, 0x35, 0x35][..], 0),
            (0xC1, &[0x14, 0x15, 0xC0][..], 0),
            (0xC2, &[0x06, 0x3A][..], 0),
            (0xC4, &[0x72, 0x12][..], 0),
            (0xBE, &[0x00][..], 0),
            (0xDE, &[0x02][..], 0),
            (0xE5, &[0x00, 0x02, 0x00][..], 0),
            (0xE5, &[0x01, 0x02, 0x00][..], 0),
            (0xDE, &[0x00][..], 0),
            (0x35, &[0x00][..], 0),
            (0x3A, &[0x05][..], 0),
            (
                0x2A,
                if cfg!(feature = "c6-landscape") {
                    &[0, 0, 1, 63][..]
                } else {
                    &[0, 34, 0, 205][..]
                },
                0,
            ),
            (
                0x2B,
                if cfg!(feature = "c6-landscape") {
                    &[0, 34, 0, 205][..]
                } else {
                    &[0, 0, 1, 63][..]
                },
                0,
            ),
            (0xDE, &[0x02][..], 0),
            (0xE5, &[0x00, 0x02, 0x00][..], 0),
            (0xDE, &[0x00][..], 0),
            (0x29, &[][..], 0),
        ] {
            self.command(cmd, data)?;
            if ms != 0 {
                delay.delay_ms(ms);
            }
        }
        self.command(0x21, &[])
    }

    pub fn write_tile(&mut self, y: u16, pixels: &[u8; TILE_BYTES]) -> Result<(), SPI::Error> {
        assert!((y as usize) < HEIGHT);
        let x: u16 = if cfg!(all(
            feature = "board-waveshare-c6",
            not(feature = "c6-landscape")
        )) {
            34
        } else {
            0
        };
        let y_offset = if cfg!(feature = "c6-landscape") {
            34
        } else {
            0
        };
        let x_end = x + WIDTH as u16 - 1;
        self.command(
            0x2A,
            &[(x >> 8) as u8, x as u8, (x_end >> 8) as u8, x_end as u8],
        )?;
        let rows = TILE_ROWS.min(HEIGHT - y as usize);
        let start = y + y_offset;
        let end = start + rows as u16 - 1;
        self.command(
            0x2B,
            &[(start >> 8) as u8, start as u8, (end >> 8) as u8, end as u8],
        )?;
        self.command(0x2C, &[])?;
        self.spi.write(&pixels[..WIDTH * rows * 2])?;
        self.spi.flush()
    }

    pub fn sleep(&mut self, delay: &mut impl DelayNs) -> Result<(), SPI::Error> {
        self.command(0x28, &[])?;
        self.command(0x10, &[])?;
        delay.delay_ms(120);
        Ok(())
    }
}

/// Clip drawing to 16 rows, stored in the panel's big-endian RGB565 wire format.
/// The board borrows these 7.5 KiB from its idle settings scratch buffer.
pub struct Tile<'a> {
    pub y: u16,
    pub pixels: &'a mut [u8; TILE_BYTES],
}

impl Dimensions for Tile<'_> {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(
            Point::new(0, self.y.into()),
            Size::new(
                WIDTH as u32,
                TILE_ROWS.min(HEIGHT.saturating_sub(self.y as usize)) as u32,
            ),
        )
    }
}

impl DrawTarget for Tile<'_> {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Infallible> {
        for Pixel(p, color) in pixels {
            if (0..WIDTH as i32).contains(&p.x)
                && (i32::from(self.y)..(i32::from(self.y) + TILE_ROWS as i32).min(HEIGHT as i32))
                    .contains(&p.y)
            {
                let offset = ((p.y - i32::from(self.y)) as usize * WIDTH + p.x as usize) * 2;
                self.pixels[offset..offset + 2]
                    .copy_from_slice(&color.into_storage().to_be_bytes());
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Rgb565) -> Result<(), Infallible> {
        let clip = area.intersection(&self.bounding_box());
        for p in clip.points() {
            let offset = ((p.y - i32::from(self.y)) as usize * WIDTH + p.x as usize) * 2;
            self.pixels[offset..offset + 2].copy_from_slice(&color.into_storage().to_be_bytes());
        }
        Ok(())
    }
}
