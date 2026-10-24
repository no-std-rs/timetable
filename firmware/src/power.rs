//! Session cancellation and idle waits own their futures: all radio resources
//! have dropped before the caller acknowledges shutdown or enters idle.
use core::future::Future;
use embassy_futures::select::{Either, select};

// Return the Select directly. An async wrapper that moves `active` into a
// second Select state can duplicate the large network future in static RAM.
pub fn session<A: Future, S: Future<Output = ()>>(
    active: A,
    shutdown: S,
) -> embassy_futures::select::Select<S, A> {
    select(shutdown, active)
}
#[derive(Debug, PartialEq, Eq)]
pub enum Wake {
    Refresh,
    Shutdown,
}
pub async fn idle(
    deadline: impl Future<Output = ()>,
    manual: impl Future<Output = ()>,
    shutdown: impl Future<Output = ()>,
) -> Wake {
    match select(shutdown, select(deadline, manual)).await {
        Either::First(()) => Wake::Shutdown,
        Either::Second(_) => Wake::Refresh,
    }
}

/// Prepare both startup slots before transferring the radio or leaking storage.
/// A failed second allocation drops the first slot and returns the same token.
pub fn prepare_startup<R, A, B, E>(
    radio: R,
    first: impl FnOnce() -> Result<A, E>,
    second: impl FnOnce() -> Result<B, E>,
) -> Result<(R, A, B), (R, E)> {
    let a = match first() {
        Ok(a) => a,
        Err(error) => return Err((radio, error)),
    };
    let b = match second() {
        Ok(b) => b,
        Err(error) => return Err((radio, error)),
    };
    Ok((radio, a, b))
}
