//! A six-hour departure cache: four bytes per entry and one shared UTC epoch.
//! Full timestamps are reconstructed by value; no packed references or heap.
use crate::model::{Departure, MAX_DEPARTURES};
use gtfs::Error;
use heapless::Vec;

pub const HORIZON_SECONDS: i64 = 6 * 3600;

#[derive(Clone, Copy)]
struct Entry {
    seconds: u16,
    selection: u8,
    headsign: u8,
}

pub struct Departures {
    epoch: i64,
    entries: Vec<Entry, MAX_DEPARTURES>,
}

impl Default for Departures {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Departures {
    pub const fn new(epoch: i64) -> Self {
        Self {
            epoch,
            entries: Vec::new(),
        }
    }

    /// Start a new snapshot, discarding entries from the previous epoch.
    pub fn reset(&mut self, epoch: i64) {
        self.clear();
        self.epoch = epoch;
    }

    /// Reject an out-of-horizon time or selection before changing the cache.
    pub fn push(&mut self, departure: Departure) -> Result<(), Error> {
        let seconds = departure
            .at
            .checked_sub(self.epoch)
            .filter(|seconds| (1..=HORIZON_SECONDS).contains(seconds))
            .ok_or(Error::InvalidField("departure time"))?;
        if usize::from(departure.group) >= crate::config::MAX_GROUPS
            || usize::from(departure.pick) >= crate::config::MAX_PICKS
        {
            return Err(Error::InvalidField("departure selection"));
        }
        self.entries
            .push(Entry {
                seconds: seconds as u16,
                selection: departure.group * 4
                    + departure.pick
                    + u8::from(departure.conditional) * 16,
                headsign: departure.headsign,
            })
            .map_err(|_| Error::IndexFull)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn truncate(&mut self, len: usize) {
        self.entries.truncate(len);
    }

    /// UTC time, group, then choice; preserve distinct simultaneous entries.
    pub fn sort(&mut self) {
        self.entries
            .sort_unstable_by_key(|entry| (entry.seconds, entry.selection & 15));
    }

    pub fn get(&self, index: usize) -> Option<Departure> {
        self.entries.get(index).map(|entry| self.decode(entry))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = Departure> + DoubleEndedIterator + '_ {
        self.entries.iter().map(|entry| self.decode(entry))
    }

    fn decode(&self, entry: &Entry) -> Departure {
        Departure {
            at: self.epoch + i64::from(entry.seconds),
            group: (entry.selection >> 2) & 3,
            pick: entry.selection & 3,
            conditional: entry.selection & 16 != 0,
            headsign: entry.headsign,
        }
    }
}

// Keep layout/capacity changes from silently defeating the compact encoding.
const _: () = assert!(core::mem::size_of::<Entry>() == 4);
const _: () = assert!(crate::config::MAX_GROUPS == 4 && crate::config::MAX_PICKS == 4);
const _: () = assert!(HORIZON_SECONDS <= u16::MAX as i64);
