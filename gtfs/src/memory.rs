//! Account async task state and explicit firmware reserves in bytes. This is a
//! budget, not a measurement of a particular board's network stack/call stack.

use crate::Error;

pub const DEVICE_RAM: usize = 400_000;

#[derive(Debug, Clone, Copy)]
pub struct Reserves {
    pub tls_rx: usize,
    pub tls_tx: usize,
    pub http: usize,
    pub network: usize,
    pub call_stack: usize,
    pub application: usize,
}

impl Reserves {
    pub const EMBEDDED: Self = Self {
        tls_rx: 16384,
        tls_tx: 4096,
        http: 4096,
        network: 32768,
        call_stack: 65536,
        application: 16384,
    };
    pub fn total(self) -> Result<usize, Error> {
        [
            self.tls_rx,
            self.tls_tx,
            self.http,
            self.network,
            self.call_stack,
            self.application,
        ]
        .into_iter()
        .try_fold(0usize, |sum, n| {
            sum.checked_add(n).ok_or(Error::RamBudgetExceeded)
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Usage {
    pub task_state: usize,
    pub external_storage: usize,
    pub reserves: usize,
    pub total: usize,
    pub remaining: usize,
}

impl Usage {
    /// Include all buffers OWNED by the future via size_of_val, then add any
    /// caller-owned buffers it BORROWS as external_storage (do not count twice).
    pub fn check<F: core::future::Future>(
        future: &F,
        external_storage: usize,
        reserves: Reserves,
        limit: usize,
    ) -> Result<Self, Error> {
        let task_state = core::mem::size_of_val(future);
        let reserves = reserves.total()?;
        let total = task_state
            .checked_add(external_storage)
            .and_then(|n| n.checked_add(reserves))
            .ok_or(Error::RamBudgetExceeded)?;
        let remaining = limit.checked_sub(total).ok_or(Error::RamBudgetExceeded)?;
        Ok(Self {
            task_state,
            external_storage,
            reserves,
            total,
            remaining,
        })
    }
}
