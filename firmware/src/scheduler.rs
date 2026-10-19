//! Cumulative poll work for one radio session; no per-packet logging or allocation.
use core::{future::Future, pin::Pin};
use embassy_time::Instant;
use portable_atomic::{AtomicU32, Ordering};
use timetable_firmware::cooperative::measure;

struct Work {
    total: AtomicU32,
    longest: AtomicU32,
}
impl Work {
    const fn new() -> Self {
        Self {
            total: AtomicU32::new(0),
            longest: AtomicU32::new(0),
        }
    }
    fn record(&self, us: u32) {
        self.total.fetch_add(us, Ordering::Relaxed);
        self.longest.fetch_max(us, Ordering::Relaxed);
    }
    fn reset(&self) {
        self.total.store(0, Ordering::Relaxed);
        self.longest.store(0, Ordering::Relaxed);
    }
}
static APPLICATION: Work = Work::new();
static RUNNER: Work = Work::new();
static DISPLAY: Work = Work::new();
fn micros() -> u32 {
    Instant::now().as_micros() as u32
}

pub fn reset() {
    APPLICATION.reset();
    RUNNER.reset();
    DISPLAY.reset();
}
pub fn application<F: Future>(future: Pin<&mut F>) -> impl Future<Output = F::Output> {
    measure(future, micros, |us| APPLICATION.record(us))
}
pub fn runner<F: Future>(future: Pin<&mut F>) -> impl Future<Output = F::Output> {
    measure(future, micros, |us| RUNNER.record(us))
}
pub fn tile(start: Instant) {
    DISPLAY.record(micros().wrapping_sub(start.as_micros() as u32));
}
pub fn report() {
    log::info!(
        "Scheduler (session): app {} ms / max poll {} us; net {} ms / max poll {} us; tiles {} ms / max tile {} us",
        APPLICATION.total.load(Ordering::Relaxed) / 1000,
        APPLICATION.longest.load(Ordering::Relaxed),
        RUNNER.total.load(Ordering::Relaxed) / 1000,
        RUNNER.longest.load(Ordering::Relaxed),
        DISPLAY.total.load(Ordering::Relaxed) / 1000,
        DISPLAY.longest.load(Ordering::Relaxed),
    );
}
