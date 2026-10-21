//! Poll each network sibling only after its own waker fires.
//! Relay tasks supply real Embassy wakers for the integrated timer queue.
use core::{
    cell::{Cell, RefCell},
    future::{Future, poll_fn},
    pin::Pin,
    task::{Context, Poll, Waker},
};
use embassy_sync::blocking_mutex::{Mutex, raw::CriticalSectionRawMutex};
use embassy_sync::waitqueue::AtomicWaker;
#[cfg(feature = "http-probe")]
use portable_atomic::AtomicU32;
#[cfg(feature = "http-probe")]
static ENABLED: AtomicBool = AtomicBool::new(true);
#[cfg(feature = "http-probe")]
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Release);
    for gate in [&NETWORK, &WORK] {
        gate.ready.store(true, Ordering::Release);
        gate.parent.wake();
    }
}
fn enabled() -> bool {
    #[cfg(feature = "http-probe")]
    {
        ENABLED.load(Ordering::Acquire)
    }
    #[cfg(not(feature = "http-probe"))]
    {
        true
    }
}
use portable_atomic::{AtomicBool, Ordering};

pub struct Gate {
    ready: AtomicBool,
    active: AtomicBool,
    parent: AtomicWaker,
    relay: Mutex<CriticalSectionRawMutex, RefCell<Option<Waker>>>,
    #[cfg(feature = "http-probe")]
    polls: AtomicU32,
    #[cfg(feature = "http-probe")]
    runs: AtomicU32,
}
impl Gate {
    pub const fn new() -> Self {
        Self {
            ready: AtomicBool::new(true),
            active: AtomicBool::new(false),
            parent: AtomicWaker::new(),
            relay: Mutex::new(RefCell::new(None)),
            #[cfg(feature = "http-probe")]
            polls: AtomicU32::new(0),
            #[cfg(feature = "http-probe")]
            runs: AtomicU32::new(0),
        }
    }
    pub async fn relay(&'static self) {
        poll_fn(|cx| {
            self.relay
                .lock(|waker| *waker.borrow_mut() = Some(cx.waker().clone()));
            if self.active.load(Ordering::Acquire) {
                self.ready.store(true, Ordering::Release);
                self.parent.wake();
            }
            Poll::<()>::Pending
        })
        .await;
    }
    #[inline(always)]
    pub fn run<F: Future>(&'static self, future: F) -> Gated<F> {
        // Keep construction trivial: callers contain large async values. Any
        // acquisition/panic path here can force stack copies before TLS starts.
        Gated {
            gate: self,
            initialized: Cell::new(false),
            future,
        }
    }
    #[cfg(feature = "http-probe")]
    pub fn reset(&self) {
        self.polls.store(0, Ordering::Relaxed);
        self.runs.store(0, Ordering::Relaxed);
    }
    #[cfg(feature = "http-probe")]
    pub fn counts(&self) -> (u32, u32) {
        (
            self.polls.load(Ordering::Relaxed),
            self.runs.load(Ordering::Relaxed),
        )
    }
}
pub struct Gated<F> {
    gate: &'static Gate,
    initialized: Cell<bool>,
    future: F,
}
impl<F: Future> Future for Gated<F> {
    type Output = F::Output;
    #[inline(never)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let gate = self.as_ref().get_ref().gate;
        if !self.as_ref().get_ref().initialized.get() {
            assert!(
                !gate.active.swap(true, Ordering::AcqRel),
                "wake gate already in use"
            );
            gate.ready.store(true, Ordering::Release);
            self.as_ref().get_ref().initialized.set(true);
        }
        #[cfg(feature = "http-probe")]
        gate.polls.fetch_add(1, Ordering::Relaxed);
        gate.parent.register(cx.waker());
        let gated = enabled();
        if gated && !gate.ready.swap(false, Ordering::AcqRel) {
            return Poll::Pending;
        }
        #[cfg(feature = "http-probe")]
        gate.runs.fetch_add(1, Ordering::Relaxed);
        let waker = if gated {
            let Some(waker) = gate.relay.lock(|waker| waker.borrow().clone()) else {
                return Poll::Pending;
            };
            waker
        } else {
            cx.waker().clone()
        };
        // `future` stays pinned with its containing Gated value.
        let future = unsafe { self.map_unchecked_mut(|this| &mut this.future) };
        future.poll(&mut Context::from_waker(&waker))
    }
}
impl<F> Drop for Gated<F> {
    fn drop(&mut self) {
        if self.initialized.get() {
            self.gate.active.store(false, Ordering::Release);
            self.gate.parent.register(Waker::noop());
        }
    }
}
pub static NETWORK: Gate = Gate::new();
pub static WORK: Gate = Gate::new();
