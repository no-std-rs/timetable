//! Host-only catalogue lookup timing; excludes initialization, drawing and I/O.
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::{alloc::System, hint::black_box, time::Instant};
use timetable_display::catalog::Catalog;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() {
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    println!("round,queries,elapsed_us,allocations,reallocations,checksum");
    for round in 0..8 {
        let region = Region::new(GLOBAL);
        let start = Instant::now();
        let mut checksum = 0usize;
        for _ in 0..20 {
            for route in 0..catalog.route_count() {
                for direction in [-1, 0, 1] {
                    let catalog = black_box(&catalog);
                    let count = catalog.route_stops(black_box(route), direction).count();
                    let last = catalog
                        .route_stops(route, direction)
                        .nth(count.saturating_sub(1))
                        .map_or(0, |(index, _)| index);
                    checksum = checksum.wrapping_add(count + last);
                }
            }
        }
        let elapsed = start.elapsed();
        let stats = region.change();
        assert_eq!(stats.allocations, 0);
        assert_eq!(stats.reallocations, 0);
        println!(
            "{round},{},{},{},{},{checksum}",
            20 * catalog.route_count() * 3 * 2,
            elapsed.as_micros(),
            stats.allocations,
            stats.reallocations
        );
    }
}
