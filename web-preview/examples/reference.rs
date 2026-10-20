//! Native pixel reference for the exact scenario/gesture sequence exercised in WASM.
use timetable_web_preview::{Frame, Simulator};
fn main() {
    let now = 1_792_301_400;
    let mut sim =
        Simulator::new(include_bytes!("../../display/tests/settings.json"), now, 0).unwrap();
    let mut frame = Frame::default();
    println!("[");
    for step in 0..22 {
        match step {
            1..=5 => {
                sim.mode(step, now);
            }
            6 => {
                sim.mode(0, now);
                sim.gesture(120, 120, 0, 0, true);
            }
            7 => {
                sim.gesture(120, 65, 0, 0, false);
            }
            8 => {
                sim.gesture(180, 80, 0, 0, false);
            }
            9 => {
                sim.gesture(180, 200, 0, 0, false);
            }
            10 => {
                sim.gesture(120, 212, 0, 0, false);
            }
            11 => {
                sim.gesture(120, 120, 0, 0, false);
            }
            12 => {
                sim.gesture(120, 24, 0, 0, false);
            }
            13 => {
                sim.gesture(120, 65, 0, 0, false);
            }
            14 => {
                sim.gesture(120, 120, 0, 0, true);
            }
            15 => {
                sim.gesture(120, 100, 0, 0, false);
            }
            16 => {
                sim.gesture(120, 70, 0, 0, false);
            }
            17 => {
                sim.gesture(120, 70, 0, 0, false);
            }
            18 => {
                sim.gesture(60, 45, 0, 0, false);
            }
            19..=21 => {
                sim.gesture(45, 24, 0, 0, false);
            }
            _ => {}
        }
        sim.draw(&mut frame, (step % 2) as u8);
        let hash = frame.0.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
        println!(
            "{}{{\"step\":{step},\"screen\":{},\"hash\":\"{hash:016x}\"}}",
            if step == 0 { "" } else { "," },
            sim.screen()
        );
    }
    println!("]");
}
