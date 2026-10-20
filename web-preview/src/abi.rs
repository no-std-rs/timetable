//! Single-threaded WASM ABI, no host imports or callbacks, and no allocator.
use super::*;
use core::cell::UnsafeCell;

struct State {
    input: [u8; BUFFER_BYTES],
    output: [u8; OUTPUT_BYTES],
    frame: Frame,
    simulator: Option<Simulator>,
}
struct Cell(UnsafeCell<State>);
// SAFETY: the built module has unshared memory, no threads, no host imports and
// no callbacks. JS calls exports synchronously and reads buffers between calls.
// Each export finishes before another can enter this private module instance.
unsafe impl Sync for Cell {}
static STATE: Cell = Cell(UnsafeCell::new(State {
    input: [0; BUFFER_BYTES],
    output: [0; OUTPUT_BYTES],
    frame: Frame([0; FRAME_BYTES]),
    simulator: None,
}));
fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    // SAFETY: single-threaded, non-reentrant exported calls as documented above.
    f(unsafe { &mut *STATE.0.get() })
}
#[unsafe(no_mangle)]
pub extern "C" fn preview_input() -> *mut u8 {
    state(|s| s.input.as_mut_ptr())
}
#[unsafe(no_mangle)]
pub extern "C" fn preview_capacity() -> usize {
    BUFFER_BYTES
}
#[unsafe(no_mangle)]
pub extern "C" fn preview_frame() -> *const u8 {
    state(|s| s.frame.0.as_ptr())
}
#[unsafe(no_mangle)]
pub extern "C" fn preview_output() -> *const u8 {
    state(|s| s.output.as_ptr())
}
#[unsafe(no_mangle)]
pub extern "C" fn preview_configure(len: usize, now: i64, group: usize) -> u32 {
    state(|s| {
        let Some(bytes) = s.input.get(..len) else {
            return 0;
        };
        let Some(simulator) = Simulator::new(bytes, now, group) else {
            return 0;
        };
        s.simulator = Some(simulator);
        1
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn preview_render(now: i64, phase: u32) -> u32 {
    state(|s| {
        let Some(sim) = &mut s.simulator else {
            return 0;
        };
        sim.tick(now);
        sim.draw(&mut s.frame, phase as u8);
        1
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn preview_snapshot() -> usize {
    state(|s| {
        s.simulator
            .as_ref()
            .and_then(|sim| sim.json(&mut s.output))
            .unwrap_or(0)
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn preview_mode(mode: u32, now: i64) -> u32 {
    state(|s| u32::from(s.simulator.as_mut().is_some_and(|sim| sim.mode(mode, now))))
}
#[unsafe(no_mangle)]
pub extern "C" fn preview_refresh(now: i64) {
    state(|s| {
        if let Some(sim) = &mut s.simulator {
            sim.complete_mock_refresh(now);
        }
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn preview_gesture(x: u32, y: u32, dx: i32, dy: i32, held: u32) -> u32 {
    if x >= 240 || y >= 240 || dx.unsigned_abs() > 480 || dy.unsigned_abs() > 480 {
        return 0;
    }
    state(|s| {
        let Some(sim) = &mut s.simulator else {
            return 0;
        };
        match sim.gesture(x as u16, y as u16, dx as i16, dy as i16, held != 0) {
            ui::Action::None => 0,
            ui::Action::Refresh => 1,
            ui::Action::Save { refresh: false } => 2,
            ui::Action::Save { refresh: true } => 3,
        }
    })
}
