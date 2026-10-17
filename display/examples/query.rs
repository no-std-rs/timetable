//! Run the exact device pipeline against a local original ZIP, without hardware.
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;
use timetable_display::{Config, catalog::Catalog, feed::Builder};
#[global_allocator]
static ALLOCATOR: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let source = std::fs::read(
        args.first()
            .ok_or("Usage: query ZIP [LINE] [STOP_CODE] [UNIX_UTC]")?,
    )?;
    let catalog =
        Catalog::new(include_bytes!("../../catalog/catalog.bin")).ok_or("Invalid catalogue")?;
    let line = args.get(1).map_or("4", String::as_str);
    let route = (0..catalog.route_count())
        .find(|&i| catalog.route(i).unwrap().line == line)
        .ok_or("Unknown line")?;
    let stop = catalog
        .route_stops(route, -1)
        .find(|(_, s)| args.get(2).is_none_or(|code| s.code == code))
        .ok_or("No matching stop on that line")?
        .1;
    let route = catalog.route(route).unwrap();
    let mut config: Config =
        serde_json_core::from_slice(include_bytes!("../tests/settings.json"))?.0;
    let pick = &mut config.groups[0].picks[0];
    pick.route = route.id.try_into().unwrap();
    pick.line = route.line.try_into().unwrap();
    pick.stop = stop.id.try_into().unwrap();
    pick.stop_code = stop.code.try_into().unwrap();
    pick.stop_name = stop.name.try_into().unwrap();
    pick.direction = -1;
    let now = args.get(3).map(|v| v.parse()).transpose()?.unwrap_or(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64,
    );
    let date = timetable_display::clock::service_date(now).ok_or("Invalid clock")?;
    let mut builder = Builder::new(date);
    let mut workspace = gtfs::zip::Workspace::new();
    futures::executor::block_on(async {});
    let begin = std::time::Instant::now();
    let region = Region::new(ALLOCATOR);
    futures::executor::block_on(async {
        builder.begin(now)?;
        builder
            .calendars(source.as_slice(), 3, &config, &mut workspace)
            .await?;
        builder
            .trips(source.as_slice(), &config, &mut workspace)
            .await?;
        builder
            .departures(source.as_slice(), &config, &mut workspace)
            .await
    })?;
    let stats = region.change();
    let elapsed = begin.elapsed();
    assert_eq!((stats.allocations, stats.reallocations), (0, 0));
    println!(
        "{} / {}: {} departures in the next six hours; {:.3} s; zero allocations",
        line,
        stop.name,
        builder.events.len(),
        elapsed.as_secs_f64()
    );
    builder.events.sort_unstable_by_key(|e| e.at);
    for event in &builder.events {
        let local = timetable_display::clock::warsaw(event.at).unwrap();
        println!(
            "{:02}:{:02} {}{}",
            local.hour(),
            local.minute(),
            builder.headsigns.get(event.headsign).unwrap_or(""),
            if event.conditional { " *" } else { "" }
        );
    }
    Ok(())
}
