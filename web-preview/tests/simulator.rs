use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;
use timetable_display::{App, Config, Screen, catalog::Catalog, model::Status, ui};
use timetable_web_preview::{Frame, Simulator};
#[global_allocator]
static ALLOC: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
const JSON: &[u8] = include_bytes!("../../display/tests/settings.json");
const NOW: i64 = 1_792_301_400;

#[test]
fn ui_drafts_save_cancel_and_clock_shims_keep_device_semantics_without_allocating() {
    let region = Region::new(ALLOC);
    let mut sim = Simulator::new(JSON, NOW, 0).unwrap();
    let mut frame = Frame::default();
    let mut native = App {
        clock: Some(NOW),
        ..App::default()
    };
    let mut config: Config = serde_json_core::from_slice(JSON).unwrap().0;
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    for (x, y, held) in [
        (120, 120, true),
        (120, 65, false),
        (180, 80, false),
        (45, 24, false),
        (120, 65, false),
        (180, 80, false),
        (180, 200, false),
    ] {
        let expected = ui::tap(&mut native, &mut config, &catalog, x, y, held);
        assert_eq!(sim.gesture(x, y, 0, 0, held), expected);
        assert!(sim.config == config);
        assert_eq!(sim.app.screen, native.screen);
        assert_eq!(sim.app.draft, native.draft);
        sim.draw(&mut frame, 0);
    }
    assert_eq!(sim.config.groups[0].picks[0].walk, 7);
    assert_eq!(sim.app.screen, Screen::Board);
    let departure = sim.app.departures.get(0).unwrap();
    sim.tick(NOW + 10);
    assert_eq!(sim.app.departures.get(0), Some(departure));
    assert_eq!(sim.app.clock, Some(NOW + 10));
    assert!(sim.mode(1, NOW));
    assert_eq!(sim.app.clock, None);
    sim.tick(NOW + 100);
    assert_eq!(sim.app.clock, None);
    assert!(sim.app.board_departures().next().is_some());
    assert!(sim.app.upcoming().next().is_none());
    sim.draw(&mut frame, 1);
    assert!(sim.mode(5, NOW));
    assert_eq!(sim.app.status, Status::Offline);
    assert_eq!(sim.app.departures.get(0), Some(departure));
    assert!(!sim.mode(100, NOW));
    let allocations = region.change();
    assert_eq!((allocations.allocations, allocations.reallocations), (0, 0));
}
