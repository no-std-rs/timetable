#![no_std]
//! Native tablet adapter: existing timetable state/queries, scalable drawing commands.

pub mod board;
pub mod feed;
pub mod motion;
pub mod snapshot;

use heapless::{String, Vec};
use serde::{Deserialize, Serialize};
use timetable_display::{Config, config::Group};

pub const SETTINGS_BYTES: usize = 16384;
pub const ZONE_BYTES: usize = 404;
pub const FRAME_BYTES: usize = 32768;
pub const STATE_BYTES: usize = 64;
pub const SNAPSHOT_BYTES: usize = 16384;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u8,
    pub feed_id: String<80>,
    pub zone: Vec<u8, ZONE_BYTES>,
    pub automatic: bool,
    pub groups: Vec<Group, 4>,
    pub keep_awake: bool,
    pub camera_motion: bool,
    pub idle_seconds: u16,
    pub launch_on_boot: bool,
}
impl Settings {
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > SETTINGS_BYTES {
            return Err("Settings are too large");
        }
        let (value, used): (Self, _) = serde_json_core::from_slice_escaped(bytes, &mut [0; 256])
            .map_err(|_| "Invalid tablet settings")?;
        if !bytes[used..].iter().all(u8::is_ascii_whitespace)
            || value.version != 1
            || !gtfs::mobility::valid_id(&value.feed_id)
            || !(30..=3600).contains(&value.idle_seconds)
            || (value.camera_motion && !value.keep_awake)
            || value.zone.is_empty()
            || !value.timezone().valid()
        {
            return Err("Invalid tablet settings");
        }
        value
            .config()
            .validate()
            .map_err(|_| "Invalid timetable groups")?;
        Ok(value)
    }
    pub fn config(&self) -> Config {
        Config {
            wifi_ssid: String::try_from("Android").unwrap(),
            wifi_password: String::new(),
            automatic: self.automatic,
            battery: false,
            groups: self.groups.clone(),
        }
    }
    pub fn timezone(&self) -> timetable_display::clock::Zone<'_> {
        timetable_display::clock::Zone::from_bytes(&self.zone)
    }
}

/// Group override and presence timestamps belong to the Rust model, stored in
/// caller-owned bytes so lifecycle changes do not require a native allocator.
#[derive(Default)]
pub struct State {
    pub group: u32,
    pub manual_until: i64,
    pub last_active_ms: i64,
}
impl State {
    pub fn load(bytes: &[u8]) -> Self {
        if bytes.len() < STATE_BYTES || &bytes[..8] != b"TTSTATE1" {
            return Self::default();
        }
        Self {
            group: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            manual_until: i64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            last_active_ms: i64::from_le_bytes(bytes[24..32].try_into().unwrap()),
        }
    }
    pub fn save(&self, bytes: &mut [u8]) -> Result<(), &'static str> {
        if bytes.len() < STATE_BYTES {
            return Err("State buffer is too small");
        }
        bytes[..STATE_BYTES].fill(0);
        bytes[..8].copy_from_slice(b"TTSTATE1");
        bytes[8..12].copy_from_slice(&self.group.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.manual_until.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.last_active_ms.to_le_bytes());
        Ok(())
    }
    pub fn active(&mut self, monotonic_ms: i64) {
        self.last_active_ms = monotonic_ms.max(1);
    }
    pub fn dim(&mut self, settings: &Settings, monotonic_ms: i64) -> bool {
        if self.last_active_ms <= 0 || monotonic_ms < self.last_active_ms {
            self.active(monotonic_ms);
        }
        settings.keep_awake
            && settings.camera_motion
            && monotonic_ms.saturating_sub(self.last_active_ms)
                >= i64::from(settings.idle_seconds) * 1000
    }
}

/// Minimal platform drawing API. Font shaping/rasterization belongs to the host;
/// all content, measurements, colors, placement and hit regions are Rust-owned.
pub trait Painter {
    type Error;
    fn rect(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        radius: i32,
        color: u32,
    ) -> Result<(), Self::Error>;
    #[allow(clippy::too_many_arguments)] // Mirrors the host text drawing primitive.
    fn text(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        size: i32,
        width: i32,
        color: u32,
        style: u32,
    ) -> Result<(), Self::Error>;
}

/// Bounds-checked little-endian display list, shared by Android and host previews.
pub struct DisplayList<'a> {
    bytes: &'a mut [u8],
    used: usize,
}
impl<'a> DisplayList<'a> {
    pub fn new(bytes: &'a mut [u8]) -> Result<Self, &'static str> {
        if bytes.len() < 24 {
            return Err("Frame buffer is too small");
        }
        bytes[..24].fill(0);
        bytes[..8].copy_from_slice(b"TTDRAW01");
        Ok(Self { bytes, used: 24 })
    }
    fn push(&mut self, data: &[u8]) -> Result<(), &'static str> {
        let end = self
            .used
            .checked_add(data.len())
            .ok_or("Frame is too large")?;
        let target = self
            .bytes
            .get_mut(self.used..end)
            .ok_or("Frame is too large")?;
        target.copy_from_slice(data);
        self.used = end;
        Ok(())
    }
    fn number(&mut self, n: u32) -> Result<(), &'static str> {
        self.push(&n.to_le_bytes())
    }
    pub fn finish(self, flags: u32, group: usize) -> usize {
        self.bytes[8..12].copy_from_slice(&(self.used as u32).to_le_bytes());
        self.bytes[12..16].copy_from_slice(&flags.to_le_bytes());
        self.bytes[16..20].copy_from_slice(&(group as u32).to_le_bytes());
        self.used
    }
}
impl Painter for DisplayList<'_> {
    type Error = &'static str;
    fn rect(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        radius: i32,
        color: u32,
    ) -> Result<(), Self::Error> {
        for n in [
            1,
            x as u32,
            y as u32,
            w as u32,
            h as u32,
            radius as u32,
            color,
        ] {
            self.number(n)?;
        }
        Ok(())
    }
    fn text(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        size: i32,
        width: i32,
        color: u32,
        style: u32,
    ) -> Result<(), Self::Error> {
        for n in [
            2,
            x as u32,
            y as u32,
            size as u32,
            width as u32,
            color,
            style,
            text.len() as u32,
        ] {
            self.number(n)?;
        }
        self.push(text.as_bytes())?;
        while !self.used.is_multiple_of(4) {
            self.push(&[0])?;
        }
        Ok(())
    }
}
