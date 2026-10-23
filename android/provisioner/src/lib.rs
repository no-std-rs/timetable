//! Browser installation policy over the async transport from adb_client PR222.
#[cfg(target_arch = "wasm32")]
mod browser;
mod protocol;
#[cfg(target_arch = "wasm32")]
pub use browser::*;
