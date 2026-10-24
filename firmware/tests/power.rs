use core::{
    cell::Cell,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};
use timetable_firmware::power::{self, Wake};

struct Resource<'a, T> {
    ready: Option<T>,
    polls: &'a Cell<u8>,
    drops: &'a Cell<u8>,
}
impl<T> Drop for Resource<'_, T> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
impl<T: Copy + Unpin> Future for Resource<'_, T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<T> {
        self.polls.set(self.polls.get() + 1);
        self.ready.map_or(Poll::Pending, Poll::Ready)
    }
}
fn ready<T>(future: impl Future<Output = T>) -> T {
    match core::pin::pin!(future)
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("fixture unexpectedly pending"),
    }
}
#[test]
fn completed_and_cancelled_sessions_release_radio_resources_before_returning() {
    for shutdown in [false, true] {
        let polls = Cell::new(0);
        let drops = Cell::new(0);
        let active = Resource {
            ready: Some(10800),
            polls: &polls,
            drops: &drops,
        };
        let outcome = ready(power::session(active, async {
            if !shutdown {
                core::future::pending::<()>().await;
            }
        }));
        let result = match outcome {
            embassy_futures::select::Either::First(()) => None,
            embassy_futures::select::Either::Second(value) => Some(value),
        };
        assert_eq!(result, if shutdown { None } else { Some(10800) });
        assert_eq!(
            drops.get(),
            1,
            "radio owner must drop before idle/shutdown acknowledgement"
        );
        assert_eq!(
            polls.get(),
            if shutdown { 0 } else { 1 },
            "pending shutdown must prevent radio reinitialization"
        );
    }
}
#[test]
fn idle_wakes_for_three_hour_deadline_manual_refresh_or_shutdown_and_drops_old_waits() {
    for event in 0..3 {
        let polls = Cell::new(0);
        let drops = Cell::new(0);
        let deadline = Resource {
            ready: (event == 0).then_some(()),
            polls: &polls,
            drops: &drops,
        };
        let manual = Resource {
            ready: (event == 1).then_some(()),
            polls: &polls,
            drops: &drops,
        };
        let shutdown = Resource {
            ready: (event == 2).then_some(()),
            polls: &polls,
            drops: &drops,
        };
        assert_eq!(
            ready(power::idle(deadline, manual, shutdown)),
            if event == 2 {
                Wake::Shutdown
            } else {
                Wake::Refresh
            }
        );
        assert_eq!(
            drops.get(),
            3,
            "old timer/manual/shutdown futures must not leak into another radio session"
        );
    }
}

#[test]
fn session_selection_does_not_duplicate_the_large_network_future() {
    let active = core::future::ready([0_u8; 32768]);
    let bytes = core::mem::size_of_val(&active);
    let selected = power::session(active, core::future::pending::<()>());
    assert!(
        core::mem::size_of_val(&selected) <= bytes + 64,
        "session wrapper duplicated a full network future"
    );
}

#[test]
fn failed_startup_keeps_radio_and_releases_provisional_slots_before_retry() {
    struct Slot<'a>(&'a Cell<u8>);
    impl Drop for Slot<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    for first_fails in [false, true] {
        let radio_drops = Cell::new(0);
        let slot_drops = Cell::new(0);
        let token = Slot(&radio_drops);
        let error = power::prepare_startup(
            token,
            || {
                if first_fails {
                    Err("first")
                } else {
                    Ok(Slot(&slot_drops))
                }
            },
            || Err::<Slot<'_>, _>("second"),
        );
        let (token, error) = match error {
            Err(failure) => failure,
            Ok(_) => panic!("failure injection succeeded"),
        };
        assert_eq!(error, if first_fails { "first" } else { "second" });
        assert_eq!(
            radio_drops.get(),
            0,
            "radio token must survive failed initialization"
        );
        assert_eq!(
            slot_drops.get(),
            u8::from(!first_fails),
            "provisional allocation must be freed"
        );
        let ready = power::prepare_startup(
            token,
            || Ok::<_, &str>(Slot(&slot_drops)),
            || Ok(Slot(&slot_drops)),
        );
        assert!(ready.is_ok(), "same radio token must be usable on retry");
        assert_eq!(radio_drops.get(), 0);
        drop(ready);
        assert_eq!(radio_drops.get(), 1);
        assert_eq!(slot_drops.get(), 2 + u8::from(!first_fails));
    }
}
