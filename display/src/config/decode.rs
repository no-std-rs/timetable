//! Fill bounded settings progressively rather than returning nested large vectors.
use super::{Config, Group, MAX_GROUPS, MAX_PICKS, Pick};
use core::fmt;
use heapless::{String, Vec};
use serde::{
    Deserialize, Deserializer,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};

const CONFIG_FIELDS: &[&str] = &[
    "wifi_ssid",
    "wifi_password",
    "automatic",
    "battery",
    "groups",
];
const GROUP_FIELDS: &[&str] = &["name", "days", "start", "end", "picks", "journey"];

fn field<E: de::Error>(seen: &mut u8, index: usize, names: &[&'static str]) -> Result<(), E> {
    if *seen & (1 << index) != 0 {
        return Err(E::duplicate_field(names[index]));
    }
    *seen |= 1 << index;
    Ok(())
}
fn required<E: de::Error>(seen: u8, mask: u8, names: &[&'static str]) -> Result<(), E> {
    for (i, name) in names.iter().enumerate() {
        if mask & (1 << i) != 0 && seen & (1 << i) == 0 {
            return Err(E::missing_field(name));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum ConfigField {
    WifiSsid,
    WifiPassword,
    Automatic,
    Battery,
    Groups,
}
#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum GroupField {
    Name,
    Days,
    Start,
    End,
    Picks,
    Journey,
}

pub(super) struct ConfigSeed<'a>(pub &'a mut Config);
impl<'de> DeserializeSeed<'de> for ConfigSeed<'_> {
    type Value = ();
    #[inline(never)]
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for ConfigSeed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("device settings")
    }
    #[inline(never)]
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let mut seen = 0;
        self.0.battery = false;
        while let Some(key) = map.next_key::<ConfigField>()? {
            let index = match key {
                ConfigField::WifiSsid => 0,
                ConfigField::WifiPassword => 1,
                ConfigField::Automatic => 2,
                ConfigField::Battery => 3,
                ConfigField::Groups => 4,
            };
            field(&mut seen, index, CONFIG_FIELDS)?;
            match key {
                ConfigField::WifiSsid => self.0.wifi_ssid = map.next_value()?,
                ConfigField::WifiPassword => self.0.wifi_password = map.next_value()?,
                ConfigField::Automatic => self.0.automatic = map.next_value()?,
                ConfigField::Battery => self.0.battery = map.next_value()?,
                ConfigField::Groups => map.next_value_seed(Groups(&mut self.0.groups))?,
            }
        }
        required(seen, 0b10111, CONFIG_FIELDS)
    }
}

struct Groups<'a>(&'a mut Vec<Group, MAX_GROUPS>);
impl<'de> DeserializeSeed<'de> for Groups<'_> {
    type Value = ();
    #[inline(never)]
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for Groups<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("up to four groups")
    }
    #[inline(never)]
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        self.0.clear();
        while let Some(group) = seq.next_element_seed(GroupSeed)? {
            self.0
                .push(group)
                .map_err(|_| de::Error::custom("too many groups"))?;
        }
        Ok(())
    }
}

struct GroupSeed;
impl<'de> DeserializeSeed<'de> for GroupSeed {
    type Value = Group;
    #[inline(never)]
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Group, D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for GroupSeed {
    type Value = Group;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a departure group")
    }
    #[inline(never)]
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Group, M::Error> {
        let mut group = Group {
            name: String::new(),
            days: 0,
            start: 0,
            end: 0,
            picks: Vec::new(),
            journey: None,
        };
        let mut seen = 0;
        while let Some(key) = map.next_key::<GroupField>()? {
            let index = match key {
                GroupField::Name => 0,
                GroupField::Days => 1,
                GroupField::Start => 2,
                GroupField::End => 3,
                GroupField::Picks => 4,
                GroupField::Journey => 5,
            };
            field(&mut seen, index, GROUP_FIELDS)?;
            match key {
                GroupField::Name => group.name = map.next_value()?,
                GroupField::Days => group.days = map.next_value()?,
                GroupField::Start => group.start = map.next_value()?,
                GroupField::End => group.end = map.next_value()?,
                GroupField::Picks => map.next_value_seed(Picks(&mut group.picks))?,
                GroupField::Journey => group.journey = map.next_value()?,
            }
        }
        required(seen, 0b11111, GROUP_FIELDS)?;
        Ok(group)
    }
}

struct Picks<'a>(&'a mut Vec<Pick, MAX_PICKS>);
impl<'de> DeserializeSeed<'de> for Picks<'_> {
    type Value = ();
    #[inline(never)]
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for Picks<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("up to four choices")
    }
    #[inline(never)]
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        self.0.clear();
        while let Some(pick) = seq.next_element::<Pick>()? {
            self.0
                .push(pick)
                .map_err(|_| de::Error::custom("too many choices"))?;
        }
        Ok(())
    }
}
