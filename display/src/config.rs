//! Bounded settings shared by the USB installer and firmware.
use heapless::{String, Vec};
use serde::{Deserialize, Serialize};
mod decode;

pub const FLASH_OFFSET: u32 = 0x3f0000;
pub const BANK_SIZE: usize = 0x8000;
pub const IMAGE_SIZE: usize = 8192;
pub const HEADER_SIZE: usize = 24;
pub const MAX_GROUPS: usize = 4;
pub const MAX_PICKS: usize = 4;
pub const MAGIC: &[u8; 8] = b"TTABLE01";

#[derive(Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub wifi_ssid: String<32>,
    pub wifi_password: String<64>,
    pub automatic: bool,
    /// Optional for settings installed before battery operation was available.
    #[serde(default, skip_serializing_if = "disabled")]
    pub battery: bool,
    pub groups: Vec<Group, MAX_GROUPS>,
}

fn disabled(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String<24>,
    /// Monday is bit 0. An empty mask disables automatic activation.
    pub days: u8,
    pub start: u16,
    pub end: u16,
    pub picks: Vec<Pick, MAX_PICKS>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journey: Option<Journey>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct JourneyStop {
    pub stop: String<32>,
    pub walk: u8,
}
impl<'de> Deserialize<'de> for JourneyStop {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            stop: String<32>,
            walk: u8,
            #[serde(default, rename = "stop_name")]
            _name: Option<serde::de::IgnoredAny>,
            #[serde(default, rename = "stop_code")]
            _code: Option<serde::de::IgnoredAny>,
        }
        let value = Wire::deserialize(deserializer)?;
        Ok(Self {
            stop: value.stop,
            walk: value.walk,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Journey {
    pub origins: Vec<JourneyStop, 4>,
    pub targets: Vec<JourneyStop, 4>,
    pub transfer_minutes: u8,
    pub max_transfers: u8,
}
impl Journey {
    pub fn validate(&self) -> Result<(), Error> {
        if self.origins.is_empty()
            || self.targets.is_empty()
            || self.transfer_minutes > 30
            || self.max_transfers > 2
        {
            return Err(Error::Invalid("Journey origins, targets or transfers"));
        }
        for stops in [&self.origins, &self.targets] {
            for (i, s) in stops.iter().enumerate() {
                if s.stop.is_empty()
                    || s.walk > 60
                    || s.stop.chars().any(char::is_control)
                    || stops[..i].iter().any(|previous| previous.stop == s.stop)
                {
                    return Err(Error::Invalid("Journey stop or walking time"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pick {
    pub route: String<32>,
    pub line: String<8>,
    pub stop: String<32>,
    pub stop_name: String<64>,
    pub stop_code: String<16>,
    pub headsign: String<64>,
    /// -1 = either direction, otherwise GTFS direction_id 0 or 1.
    pub direction: i8,
    pub walk: u8,
    pub spread: u8,
    pub margin: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Missing,
    Corrupt,
    Capacity,
    Invalid(&'static str),
}

impl Config {
    /// Empty provisional storage; fill and validate it before application use.
    pub const fn empty() -> Self {
        Self {
            wifi_ssid: String::new(),
            wifi_password: String::new(),
            automatic: false,
            battery: false,
            groups: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.wifi_ssid.is_empty() || self.wifi_ssid.contains('\0') {
            return Err(Error::Invalid("Wi-Fi name"));
        }
        let p = &self.wifi_password;
        if p.contains('\0')
            || !(p.is_empty()
                || (8..=63).contains(&p.len())
                || p.len() == 64 && p.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(Error::Invalid("Wi-Fi password"));
        }
        if self.groups.is_empty() {
            return Err(Error::Invalid("Add a group"));
        }
        for g in &self.groups {
            if g.name.trim().is_empty()
                || g.name.chars().any(char::is_control)
                || g.days > 127
                || g.start >= 1440
                || g.end >= 1440
            {
                return Err(Error::Invalid("Group"));
            }
            for p in &g.picks {
                p.validate()?;
            }
            if let Some(journey) = &g.journey {
                journey.validate()?;
            }
        }
        Ok(())
    }
}

impl Clone for Config {
    fn clone(&self) -> Self {
        let mut output = Self::empty();
        output.clone_from(self);
        output
    }

    // Copy one group at a time into existing storage. Returning a complete
    // nested Config temporary consumes several KiB in async task poll frames.
    #[inline(never)]
    fn clone_from(&mut self, source: &Self) {
        self.wifi_ssid.clone_from(&source.wifi_ssid);
        self.wifi_password.clone_from(&source.wifi_password);
        self.automatic = source.automatic;
        self.battery = source.battery;
        self.groups.clear();
        for group in &source.groups {
            self.groups
                .push(Group {
                    name: group.name.clone(),
                    days: group.days,
                    start: group.start,
                    end: group.end,
                    picks: group.picks.clone(),
                    journey: group.journey.clone(),
                })
                .unwrap();
        }
    }
}

impl Pick {
    pub fn validate(&self) -> Result<(), Error> {
        let id = |s: &str| !s.is_empty() && !s.chars().any(char::is_control);
        if !id(&self.route)
            || !id(&self.stop)
            || !id(&self.line)
            || self.stop_name.chars().any(char::is_control)
            || self.stop_code.chars().any(char::is_control)
            || self.headsign.chars().any(char::is_control)
            || !(-1..=1).contains(&self.direction)
            || self.walk > 60
            || self.spread > 15
            || self.margin > 10
        {
            return Err(Error::Invalid("Route, stop or walking time"));
        }
        Ok(())
    }
}

impl Group {
    pub fn departure_picks(&self) -> &[Pick] {
        if self.journey.is_some() {
            &[]
        } else {
            &self.picks
        }
    }
    pub fn active(&self, weekday: u8, minute: u16) -> bool {
        if self.start == self.end {
            return self.days & (1 << weekday) != 0;
        }
        if self.start < self.end {
            self.days & (1 << weekday) != 0 && (self.start..self.end).contains(&minute)
        } else if minute >= self.start {
            self.days & (1 << weekday) != 0
        } else {
            minute < self.end && self.days & (1 << ((weekday + 6) % 7)) != 0
        }
    }
}

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// CRC includes the generation and length; incomplete writes never become valid.
pub fn encode(config: &Config, generation: u32, out: &mut [u8; IMAGE_SIZE]) -> Result<(), Error> {
    config.validate()?;
    out.fill(0xff);
    let len =
        serde_json_core::to_slice(config, &mut out[HEADER_SIZE..]).map_err(|_| Error::Capacity)?;
    out[..8].copy_from_slice(MAGIC);
    out[8..12].copy_from_slice(&generation.to_le_bytes());
    out[12..16].copy_from_slice(&(len as u32).to_le_bytes());
    out[16..20].fill(0);
    out[20..24].fill(0);
    let checksum = crc32(&out[..HEADER_SIZE + len]);
    out[16..20].copy_from_slice(&checksum.to_le_bytes());
    Ok(())
}

#[inline(never)]
pub fn decode(bytes: &mut [u8; IMAGE_SIZE]) -> Result<(Config, u32), Error> {
    let mut config = Config::default();
    let generation = decode_into(bytes, &mut config)?;
    Ok((config, generation))
}

/// Decode into provisional caller-owned settings with bounded parser frames.
/// On error `config` is incomplete and must not be published or used.
#[inline(never)]
pub fn decode_into(bytes: &mut [u8; IMAGE_SIZE], config: &mut Config) -> Result<u32, Error> {
    if &bytes[..8] != MAGIC {
        return Err(Error::Missing);
    }
    let generation = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    if len == 0 || len > IMAGE_SIZE - HEADER_SIZE || bytes[20..24] != [0; 4] {
        return Err(Error::Corrupt);
    }
    let expected = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    bytes[16..20].fill(0);
    let actual = crc32(&bytes[..HEADER_SIZE + len]);
    bytes[16..20].copy_from_slice(&expected.to_le_bytes());
    if actual != expected {
        return Err(Error::Corrupt);
    }
    let mut escaped = [0; 64];
    let mut deserializer = serde_json_core::de::Deserializer::new(
        &bytes[HEADER_SIZE..HEADER_SIZE + len],
        Some(&mut escaped),
    );
    serde::de::DeserializeSeed::deserialize(decode::ConfigSeed(config), &mut deserializer)
        .map_err(|_| Error::Corrupt)?;
    let used = deserializer.end().map_err(|_| Error::Corrupt)?;
    if used != len {
        return Err(Error::Corrupt);
    }
    config.validate()?;
    Ok(generation)
}

/// Generations can wrap; their distance must remain below half the u32 range.
pub fn newer(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}
