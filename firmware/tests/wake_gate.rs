#[allow(dead_code)]
#[path = "../src/wake_gate.rs"]
mod wake_gate;

use core::{
    future::{Future, poll_fn},
    pin::pin,
    task::{Context, Poll, Waker},
};
use embassy_futures::select::{Either, select};
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use wake_gate::Gate;

#[derive(Default)]
struct Wakes(AtomicUsize);
impl std::task::Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn shared_select_waker_repolls_a_socket_that_only_woke_the_runner() {
    let network_waker = RefCell::new(None::<Waker>);
    let network_polls = Cell::new(0);
    let read_polls = Cell::new(0);
    let network = poll_fn(|cx| {
        network_polls.set(network_polls.get() + 1);
        *network_waker.borrow_mut() = Some(cx.waker().clone());
        Poll::<()>::Pending
    });
    let read = poll_fn(|_| {
        read_polls.set(read_polls.get() + 1);
        // embassy-net's socket with_mut wakes the stack even on an empty read.
        network_waker.borrow().as_ref().unwrap().wake_by_ref();
        Poll::<()>::Pending
    });
    let mut future = pin!(select(network, read));
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes.clone());
    let mut cx = Context::from_waker(&waker);
    for _ in 0..20 {
        assert!(future.as_mut().poll(&mut cx).is_pending());
    }
    assert_eq!((network_polls.get(), read_polls.get()), (20, 20));
    assert_eq!(wakes.0.load(Ordering::Relaxed), 20);
}

#[test]
fn relay_wakers_stop_empty_read_spinning_and_forward_received_data() {
    static NETWORK: Gate = Gate::new();
    static WORK: Gate = Gate::new();
    let network_wakes = Arc::new(Wakes::default());
    let work_wakes = Arc::new(Wakes::default());
    let network_relay_waker = Waker::from(network_wakes.clone());
    let work_relay_waker = Waker::from(work_wakes.clone());
    let mut network_cx = Context::from_waker(&network_relay_waker);
    let mut work_cx = Context::from_waker(&work_relay_waker);
    let mut network_relay = pin!(NETWORK.relay());
    let mut work_relay = pin!(WORK.relay());
    assert!(network_relay.as_mut().poll(&mut network_cx).is_pending());
    assert!(work_relay.as_mut().poll(&mut work_cx).is_pending());

    let network_waker = RefCell::new(None::<Waker>);
    let read_waker = RefCell::new(None::<Waker>);
    let network_polls = Cell::new(0);
    let read_polls = Cell::new(0);
    let received = Cell::new(false);
    let network = poll_fn(|cx| {
        network_polls.set(network_polls.get() + 1);
        *network_waker.borrow_mut() = Some(cx.waker().clone());
        Poll::<()>::Pending
    });
    let read = poll_fn(|cx| {
        read_polls.set(read_polls.get() + 1);
        if received.get() {
            return Poll::Ready(42);
        }
        *read_waker.borrow_mut() = Some(cx.waker().clone());
        network_waker.borrow().as_ref().unwrap().wake_by_ref();
        Poll::Pending
    });
    let mut future = pin!(select(NETWORK.run(network), WORK.run(read)));
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes.clone());
    let mut cx = Context::from_waker(&waker);
    assert!(future.as_mut().poll(&mut cx).is_pending());
    assert_eq!(network_wakes.0.load(Ordering::Relaxed), 1);
    assert_eq!(wakes.0.load(Ordering::Relaxed), 0);
    assert!(network_relay.as_mut().poll(&mut network_cx).is_pending());
    for _ in 0..20 {
        assert!(future.as_mut().poll(&mut cx).is_pending());
    }
    assert_eq!((network_polls.get(), read_polls.get()), (2, 1));
    assert_eq!(wakes.0.load(Ordering::Relaxed), 1);

    received.set(true);
    read_waker.borrow().as_ref().unwrap().wake_by_ref();
    assert_eq!(work_wakes.0.load(Ordering::Relaxed), 1);
    assert!(work_relay.as_mut().poll(&mut work_cx).is_pending());
    assert!(matches!(
        future.as_mut().poll(&mut cx),
        Poll::Ready(Either::Second(42))
    ));
    assert_eq!((network_polls.get(), read_polls.get()), (2, 2));
}

#[test]
fn self_wakes_preserve_pinning_and_do_not_copy_large_futures() {
    use core::{marker::PhantomPinned, pin::Pin};
    static WORK: Gate = Gate::new();
    struct Large<'a> {
        data: [u8; 32768],
        address: &'a Cell<usize>,
        polls: &'a Cell<u8>,
        _pin: PhantomPinned,
    }
    impl Future for Large<'_> {
        type Output = u8;
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u8> {
            let this = self.as_ref().get_ref();
            let address = this as *const Self as usize;
            if this.address.get() == 0 {
                this.address.set(address);
            } else {
                assert_eq!(this.address.get(), address);
            }
            this.polls.set(this.polls.get() + 1);
            if this.polls.get() == 4 {
                Poll::Ready(this.data[32767])
            } else {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes);
    let mut cx = Context::from_waker(&waker);
    let mut relay = pin!(WORK.relay());
    assert!(relay.as_mut().poll(&mut cx).is_pending());
    let address = Cell::new(0);
    let polls = Cell::new(0);
    let original = Large {
        data: [7; 32768],
        address: &address,
        polls: &polls,
        _pin: PhantomPinned,
    };
    let bytes = core::mem::size_of_val(&original);
    let wrapped = WORK.run(original);
    assert!(core::mem::size_of_val(&wrapped) <= bytes + 16);
    let mut future = pin!(wrapped);
    for _ in 0..3 {
        assert!(future.as_mut().poll(&mut cx).is_pending());
        assert!(relay.as_mut().poll(&mut cx).is_pending());
    }
    assert_eq!(future.as_mut().poll(&mut cx), Poll::Ready(7));
    assert_eq!(polls.get(), 4);
}

#[test]
fn cancellation_releases_parent_and_allows_reuse_after_late_wakes() {
    static WORK: Gate = Gate::new();
    let relay_wakes = Arc::new(Wakes::default());
    let relay_waker = Waker::from(relay_wakes);
    let mut relay_cx = Context::from_waker(&relay_waker);
    let mut relay = pin!(WORK.relay());
    assert!(relay.as_mut().poll(&mut relay_cx).is_pending());
    let parent = Arc::new(Wakes::default());
    let parent_waker = Waker::from(parent.clone());
    let baseline_refs = Arc::strong_count(&parent);
    let mut cx = Context::from_waker(&parent_waker);
    let saved = RefCell::new(None::<Waker>);
    let dropped = Cell::new(false);
    struct Guard<'a>(&'a Cell<bool>);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    {
        let guard = Guard(&dropped);
        let future = async {
            let _guard = guard;
            poll_fn(|cx| {
                *saved.borrow_mut() = Some(cx.waker().clone());
                Poll::<()>::Pending
            })
            .await;
        };
        let mut future = pin!(WORK.run(future));
        assert!(future.as_mut().poll(&mut cx).is_pending());
        assert!(Arc::strong_count(&parent) > baseline_refs);
    }
    assert!(dropped.get());
    assert_eq!(Arc::strong_count(&parent), baseline_refs);
    saved.borrow().as_ref().unwrap().wake_by_ref();
    assert!(relay.as_mut().poll(&mut relay_cx).is_pending());
    assert_eq!(parent.0.load(Ordering::Relaxed), 0);
    let mut reused = pin!(WORK.run(core::future::ready(7)));
    assert_eq!(reused.as_mut().poll(&mut cx), Poll::Ready(7));
}

#[test]
fn relay_starting_after_the_first_parent_poll_resumes_the_child() {
    static WORK: Gate = Gate::new();
    let parent = Arc::new(Wakes::default());
    let parent_waker = Waker::from(parent.clone());
    let mut parent_cx = Context::from_waker(&parent_waker);
    let mut future = pin!(WORK.run(core::future::ready(42)));
    assert!(future.as_mut().poll(&mut parent_cx).is_pending());
    let mut relay = pin!(WORK.relay());
    let mut relay_cx = Context::from_waker(Waker::noop());
    assert!(relay.as_mut().poll(&mut relay_cx).is_pending());
    assert_eq!(parent.0.load(Ordering::Relaxed), 1);
    assert_eq!(future.as_mut().poll(&mut parent_cx), Poll::Ready(42));
}
