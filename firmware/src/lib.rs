#![no_std]
pub mod battery;
pub mod cache;
pub mod cooperative;
pub mod download;
pub mod panel;
pub mod power;
pub mod settings;
pub mod sources;

#[cfg(all(feature = "board-guition", feature = "board-waveshare-c6"))]
compile_error!("Select exactly one board; use --no-default-features for Waveshare C6.");
#[cfg(all(
    feature = "firmware",
    not(any(feature = "board-guition", feature = "board-waveshare-c6"))
))]
compile_error!("A firmware build requires a board feature.");
pub mod touch;
