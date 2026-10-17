#![no_std]
//! Device-independent, allocation-free model and renderer for the round display.
pub mod catalog;
pub mod clock;
pub mod config;
pub mod feed;
pub mod model;
pub mod render;
pub mod ui;

pub use config::{Config, Group, Pick};
pub use model::{App, Departure, Screen};
