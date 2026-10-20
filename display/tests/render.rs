use core::convert::Infallible;
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;
use timetable_display::{App, Config, Departure, Screen, catalog::Catalog, render};

#[global_allocator]
static ALLOC: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

// Count raster work for an empty band, without allocating a framebuffer.
struct Band {
    pixels: usize,
}
impl Dimensions for Band {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::new(0, 35), Size::new(240, 6))
    }
}
impl DrawTarget for Band {
    type Color = Rgb565;
    type Error = Infallible;
    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Infallible> {
        self.pixels += pixels.into_iter().count();
        Ok(())
    }
    fn fill_solid(&mut self, _: &Rectangle, _: Rgb565) -> Result<(), Infallible> {
        Ok(())
    }
}

#[test]
fn clipped_board_skips_offscreen_glyphs_without_allocating() {
    let c: Config = serde_json_core::from_slice(include_bytes!("settings.json"))
        .unwrap()
        .0;
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let mut app = App {
        clock: Some(100),
        ..App::default()
    };
    app.departures.reset(100);
    app.departures
        .push(Departure {
            at: 820,
            group: 0,
            pick: 0,
            conditional: false,
            headsign: 0,
        })
        .unwrap();
    app.freshness = timetable_display::freshness::Freshness {
        checked_at: Some(90),
        source_updated_at: Some(80),
    };
    for screen in [Screen::Board, Screen::Freshness] {
        app.screen = screen;
        let mut band = Band { pixels: 0 };
        let region = Region::new(ALLOC);
        render::draw(&mut band, &app, &c, &catalog).unwrap();
        let allocations = region.change();
        assert_eq!(allocations.allocations, 0);
        assert_eq!(allocations.reallocations, 0);
        assert_eq!(allocations.bytes_allocated, 0);
        // The gold seam can intersect this band; all glyphs lie outside it.
        assert!(
            band.pixels < 100,
            "rasterized {} pixels for an empty band",
            band.pixels
        );
    }
    app.screen = Screen::Board;
    app.clock = None;
    for status in [
        timetable_display::model::Status::Restoring,
        timetable_display::model::Status::Connecting,
        timetable_display::model::Status::Syncing,
        timetable_display::model::Status::Loading,
        timetable_display::model::Status::Ready,
    ] {
        app.status = status;
        let mut first = SpinnerBand(0);
        let mut second = SpinnerBand(0);
        let region = Region::new(ALLOC);
        render::draw_animated(&mut first, &app, &c, &catalog, 0).unwrap();
        render::draw_animated(&mut second, &app, &c, &catalog, 1).unwrap();
        let allocations = region.change();
        assert_eq!(allocations.allocations, 0);
        if status == timetable_display::model::Status::Ready {
            assert_eq!(first.0, second.0);
        } else {
            assert_ne!(first.0, second.0, "spinner must animate before UTC exists");
        }
    }
}

// Hash just the activity-indicator region without an allocated framebuffer.
struct SpinnerBand(u64);
impl Dimensions for SpinnerBand {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::new(43, 40), Size::new(13, 16))
    }
}
impl DrawTarget for SpinnerBand {
    type Color = Rgb565;
    type Error = Infallible;
    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Infallible> {
        for Pixel(p, c) in pixels {
            if self.bounding_box().contains(p) {
                let bits = u64::from(c.r()) * 2048 + u64::from(c.g()) * 32 + u64::from(c.b());
                self.0 = self.0.wrapping_mul(1099511628211)
                    ^ bits
                    ^ ((p.x as u64) << 24)
                    ^ ((p.y as u64) << 32);
            }
        }
        Ok(())
    }
}
