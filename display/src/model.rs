use crate::{Config, Pick, clock, departures::Departures};
use heapless::Vec;

#[derive(Default)]
pub struct Headings {
    bytes: Vec<u8, 1024>,
    spans: Vec<(u16, u8), 32>,
}
impl Headings {
    pub(crate) fn checkpoint(&self) -> (usize, usize) {
        (self.bytes.len(), self.spans.len())
    }
    pub(crate) fn restore(&mut self, (bytes, spans): (usize, usize)) {
        self.bytes.truncate(bytes);
        self.spans.truncate(spans);
    }
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.spans.clear();
    }
    pub fn get(&self, index: u8) -> Option<&str> {
        let &(offset, len) = self.spans.get(index as usize)?;
        core::str::from_utf8(&self.bytes[offset as usize..offset as usize + len as usize]).ok()
    }
    pub fn insert(&mut self, name: &str) -> Result<u8, gtfs::Error> {
        let mut end = name.len().min(64);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        let name = &name[..end];
        if let Some(index) = (0..self.spans.len()).position(|i| self.get(i as u8) == Some(name)) {
            return Ok(index as u8);
        }
        if self.spans.is_full() || self.bytes.len() + name.len() > self.bytes.capacity() {
            return Err(gtfs::Error::IndexFull);
        }
        let offset = self.bytes.len() as u16;
        self.bytes
            .extend_from_slice(name.as_bytes())
            .map_err(|_| gtfs::Error::IndexFull)?;
        self.spans
            .push((offset, name.len() as u8))
            .map_err(|_| gtfs::Error::IndexFull)?;
        Ok((self.spans.len() - 1) as u8)
    }
}

pub const MAX_DEPARTURES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Departure {
    pub at: i64,
    pub group: u8,
    pub pick: u8,
    pub conditional: bool,
    pub headsign: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Board,
    Groups,
    Picks,
    Routes,
    Stops,
    Walk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Setup,
    Connecting,
    Syncing,
    Loading,
    Ready,
    Offline,
    Failed,
    Capacity,
}

pub struct App {
    pub screen: Screen,
    pub group: usize,
    pub cursor: usize,
    pub edit_pick: usize,
    pub selected_route: usize,
    pub direction: i8,
    pub draft: Option<Pick>,
    pub manual_until: i64,
    pub status: Status,
    pub save_failed: bool,
    pub sleep_failed: bool,
    pub clock: Option<i64>,
    pub loaded_at: Option<i64>,
    pub departures: Departures,
    pub headsigns: Headings,
}

impl Default for App {
    fn default() -> Self {
        Self {
            screen: Screen::Board,
            group: 0,
            cursor: 0,
            edit_pick: 0,
            selected_route: 0,
            direction: -1,
            draft: None,
            manual_until: 0,
            status: Status::Connecting,
            save_failed: false,
            sleep_failed: false,
            clock: None,
            loaded_at: None,
            departures: Departures::new(0),
            headsigns: Headings::default(),
        }
    }
}

impl App {
    pub fn can_retry(&self) -> bool {
        !self.save_failed && matches!(self.status, Status::Offline | Status::Failed)
    }

    pub fn tick(&mut self, config: &Config, now: i64) {
        self.clock = Some(now);
        if self.manual_until == i64::MAX {
            self.manual_until = now.saturating_add(2 * 3600);
        }
        if self.screen != Screen::Board
            || now < self.manual_until
            || !config.automatic
            || !config.groups.iter().any(|g| g.days != 0)
        {
            return;
        }
        if let Some(local) = clock::warsaw(now) {
            let day = local.weekday().number_days_from_monday();
            let minute = u16::from(local.hour()) * 60 + u16::from(local.minute());
            self.group = config
                .groups
                .iter()
                .position(|g| g.active(day, minute))
                .unwrap_or(0);
        }
    }

    pub fn choose_group(&mut self, index: usize, config: &Config) {
        if index < config.groups.len() {
            self.group = index;
            self.manual_until = self
                .clock
                .map_or(i64::MAX, |now| now.saturating_add(2 * 3600));
            self.cursor = 0;
            self.screen = Screen::Board;
        }
    }

    /// Keep the previous validated snapshot on failed refreshes. Publishing is
    /// one operation, after both the GTFS footer and all capacities are checked.
    pub fn publish(&mut self, mut events: Departures, headsigns: Headings, now: i64) {
        events.sort();
        self.departures = events;
        self.headsigns = headsigns;
        self.loaded_at = Some(now);
        self.status = Status::Ready;
    }

    pub fn upcoming(&self) -> impl Iterator<Item = Departure> {
        let now = self.clock.unwrap_or(i64::MAX);
        self.departures
            .iter()
            .filter(move |d| d.group as usize == self.group && d.at > now)
    }
}

pub fn leave_in(departure: i64, now: i64, pick: &Pick) -> i64 {
    departure - now - i64::from(pick.walk + pick.margin) * 60
}

/// A walking-time model, not observed vehicle reliability. Walking duration is
/// uniform in [max(0, walk-spread), walk+spread]. Vehicle time is scheduled.
pub fn walking_chance(departure: i64, now: i64, pick: &Pick) -> u8 {
    let available = departure - now - i64::from(pick.margin) * 60;
    let low = i64::from(pick.walk.saturating_sub(pick.spread)) * 60;
    let high = i64::from(pick.walk + pick.spread) * 60;
    if available < low {
        0
    } else if available >= high {
        100
    } else {
        ((available - low) * 100 / (high - low)) as u8
    }
}
