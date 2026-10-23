#![no_std]
//! Link-time symbol declarations only. This file is NEVER packaged in the APK.
//! The resulting DT_NEEDED entry names Android's real Bionic libc.so.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop { core::hint::spin_loop(); } }
macro_rules! symbol { ($($name:ident),*) => { $(#[unsafe(no_mangle)] pub extern "C" fn $name() {})* }; }
symbol!(abort, memcpy, memmove, memset, memcmp, strlen);
