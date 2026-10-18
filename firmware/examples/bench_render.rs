//! Host CPU benchmark of the real 15-tile renderer, excluding SPI and networking.
use std::{hint::black_box, time::Instant};
use timetable_display::{App, Departure, Screen, catalog::Catalog, config, model::Status, render};
use timetable_firmware::panel::{TILE_BYTES, TILE_ROWS, Tile};

fn main() {
    let frames: u32 = std::env::args()
        .nth(1)
        .map_or(128, |n| n.parse().expect("frame count"));
    assert!(frames > 0);
    let mut image = *include_bytes!("../../display/tests/settings.bin");
    let settings = config::decode(&mut image).unwrap().0;
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let now = 1_792_301_400;
    let mut app = App {
        clock: Some(now),
        status: Status::Ready,
        draft: Some(settings.groups[0].picks[0].clone()),
        ..App::default()
    };
    app.departures.reset(now);
    for i in 0..512 {
        app.departures
            .push(Departure {
                at: now + 60 + i * 30,
                group: 0,
                pick: 0,
                conditional: false,
                headsign: 0,
            })
            .unwrap();
    }
    let mut pixels = [0; TILE_BYTES];
    println!("screen,frames,ns_per_frame,checksum");
    for screen in [
        Screen::Board,
        Screen::Groups,
        Screen::Picks,
        Screen::Routes,
        Screen::Stops,
        Screen::Walk,
    ] {
        app.screen = screen;
        let mut hash = 0xcbf29ce484222325u64;
        // Pixel checksum is outside the timed region, once per screen.
        for y in (0..240).step_by(TILE_ROWS) {
            render::draw(
                &mut Tile {
                    y,
                    pixels: &mut pixels,
                },
                &app,
                &settings,
                &catalog,
            )
            .unwrap();
            for byte in &pixels {
                hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
            }
        }
        let start = Instant::now();
        for _ in 0..frames {
            for y in (0..240).step_by(TILE_ROWS) {
                render::draw(
                    &mut Tile {
                        y,
                        pixels: &mut pixels,
                    },
                    black_box(&app),
                    black_box(&settings),
                    black_box(&catalog),
                )
                .unwrap();
                black_box(&pixels);
            }
        }
        println!(
            "{screen:?},{frames},{},{hash:016x}",
            start.elapsed().as_nanos() / u128::from(frames)
        );
    }
}
