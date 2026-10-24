#![no_std]
//! Device-independent, allocation-free model and renderer for the round display.
pub mod catalog;
pub mod clock;
pub mod config;
pub mod departures;
pub mod feed;
pub mod freshness;
pub mod journey;
pub mod model;
pub mod profile;
pub mod render;
pub mod ui;

pub use config::{Config, Group, Pick};
pub use model::{App, Departure, Screen};
