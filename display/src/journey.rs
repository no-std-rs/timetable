//! Exact, bounded timetable journeys: access walk, up to three rides, egress walk.
//! The device builds a departure/arrival profile for up to six hours. Rendering
//! only selects among that profile; it never asks a routing service for a path.
//!
//! Storage holds only calls that can matter: first the calls at the chosen
//! places, then calls at places that both a ride from a start and a ride towards
//! a target pass. If that still does not fit, the window ends earlier and the
//! profile records its actual end; a shortened profile is never presented as a
//! complete six-hour search.
use crate::{clock, config::Journey, feed::Tables};
use gtfs::{Date, Error, Id, StopTime, Table, csv::Column as C, zip};
use heapless::{String, Vec};

pub const HORIZON: i64 = 6 * 3600;
pub const MAX_PATHS: usize = 64;
const GROUPS: usize = crate::config::MAX_GROUPS;
/// A shorter profile would not reliably cover the next connection.
const MIN_WINDOW: u16 = 3600;
const NONE: u16 = u16::MAX;
const UNREADY: u16 = 0x7fff;
const FINISH: u16 = 0x8000;
/// Ride.day keeps the service day in its low bits and request stops above.
const DAY: u8 = 0x03;
const REQUEST_BOARD: u8 = 0x80;
const REQUEST_ALIGHT: u8 = 0x40;
/// Leg flags: the rider has to signal the driver or arrange the stop.
pub const LEG_REQUEST_BOARD: u8 = 1;
pub const LEG_REQUEST_ALIGHT: u8 = 2;
/// Group limits: the window ended early to fit, or fewer transfers were searched.
pub const LIMIT_WINDOW: u8 = 1;
pub const LIMIT_TRANSFERS: u8 = 2;
/// Firmware reuses the departure parser's 16 KiB ID arena for this many calls.
pub const DEVICE_CALLS: usize = 1170;
const ANDROID_CALLS: usize = 3072;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Leg {
    pub line: u8,
    pub from: u8,
    pub to: u8,
    pub flags: u8,
    pub departure: u16,
    pub arrival: u16,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Path {
    pub group: u8,
    pub origin: u8,
    pub target: u8,
    pub walk: u8,
    pub egress: u8,
    pub len: u8,
    /// Latest time to leave home, relative to Cache::epoch.
    pub leave: u16,
    pub arrival: u16,
    pub legs: [Leg; 3],
}
#[derive(Clone, Default)]
pub struct Cache {
    pub epoch: i64,
    pub until: i64,
    /// Seconds after epoch for which each group's profile is complete.
    pub windows: [u16; GROUPS],
    pub limits: [u8; GROUPS],
    pub paths: Vec<Path, MAX_PATHS>,
}
impl Cache {
    pub fn reset(&mut self, now: i64) {
        self.epoch = now;
        self.until = now + HORIZON;
        self.windows = [HORIZON as u16; GROUPS];
        self.limits = [0; GROUPS];
        self.paths.clear();
    }
    /// End of the complete profile for one group.
    pub fn until_for(&self, group: usize) -> i64 {
        self.windows
            .get(group)
            .map_or(self.until, |&w| self.until.min(self.epoch + i64::from(w)))
    }
    pub fn limits_for(&self, group: usize) -> u8 {
        self.limits.get(group).copied().unwrap_or(0)
    }
    pub fn best(&self, group: usize, now: i64) -> Option<&Path> {
        if now < self.epoch || now > self.until_for(group) {
            return None;
        }
        self.paths
            .iter()
            .filter(|p| p.group as usize == group && self.epoch + i64::from(p.leave) >= now)
            .min_by_key(|p| (p.arrival, p.len, p.walk, core::cmp::Reverse(p.leave)))
    }
    /// What to take after missing `after`: the best connection leaving later.
    pub fn next_after(&self, group: usize, now: i64, after: &Path) -> Option<&Path> {
        self.best(group, now.max(self.epoch + i64::from(after.leave) + 1))
    }
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, &'static str> {
        let mut at = 0;
        let mut put = |bytes: &[u8]| -> Result<(), &'static str> {
            output
                .get_mut(at..at + bytes.len())
                .ok_or("Journey snapshot buffer too small")?
                .copy_from_slice(bytes);
            at += bytes.len();
            Ok(())
        };
        put(b"TTJNY003")?;
        put(&self.epoch.to_le_bytes())?;
        put(&self.until.to_le_bytes())?;
        for window in self.windows {
            put(&window.to_le_bytes())?;
        }
        put(&self.limits)?;
        put(&[self.paths.len() as u8, 0])?;
        for path in &self.paths {
            put(&[
                path.group,
                path.origin,
                path.target,
                path.walk,
                path.egress,
                path.len,
            ])?;
            put(&path.leave.to_le_bytes())?;
            put(&path.arrival.to_le_bytes())?;
            for leg in path.legs {
                put(&[leg.line, leg.from, leg.to, leg.flags])?;
                put(&leg.departure.to_le_bytes())?;
                put(&leg.arrival.to_le_bytes())?;
            }
        }
        Ok(at)
    }
    pub fn decode(
        input: &[u8],
        config: &crate::Config,
        labels: &crate::model::Headings,
    ) -> Result<Self, &'static str> {
        let invalid = "Invalid journey snapshot";
        let mut at = 0;
        let mut take = |len: usize| -> Result<&[u8], &'static str> {
            let bytes = input.get(at..at + len).ok_or(invalid)?;
            at += len;
            Ok(bytes)
        };
        if take(8)? != b"TTJNY003" {
            return Err(invalid);
        }
        let epoch = i64::from_le_bytes(take(8)?.try_into().unwrap());
        let until = i64::from_le_bytes(take(8)?.try_into().unwrap());
        if !clock::valid_utc(epoch) || until.checked_sub(epoch) != Some(HORIZON) {
            return Err(invalid);
        }
        let mut cache = Self::default();
        cache.reset(epoch);
        for window in &mut cache.windows {
            *window = u16::from_le_bytes(take(2)?.try_into().unwrap());
            if i64::from(*window) > HORIZON {
                return Err(invalid);
            }
        }
        cache.limits.copy_from_slice(take(GROUPS)?);
        let counts = take(2)?;
        if counts[1] != 0
            || cache
                .limits
                .iter()
                .any(|&l| l & !(LIMIT_WINDOW | LIMIT_TRANSFERS) != 0)
        {
            return Err(invalid);
        }
        for _ in 0..counts[0] {
            let row = take(10)?;
            let mut path = Path {
                group: row[0],
                origin: row[1],
                target: row[2],
                walk: row[3],
                egress: row[4],
                len: row[5],
                leave: u16::from_le_bytes(row[6..8].try_into().unwrap()),
                arrival: u16::from_le_bytes(row[8..10].try_into().unwrap()),
                ..Path::default()
            };
            let journey = config
                .groups
                .get(path.group as usize)
                .and_then(|g| g.journey.as_ref())
                .ok_or(invalid)?;
            let origin = journey.origins.get(path.origin as usize).ok_or(invalid)?;
            let target = journey.targets.get(path.target as usize).ok_or(invalid)?;
            if path.walk != origin.walk
                || path.egress != target.walk
                || path.len == 0
                || path.len > journey.max_transfers + 1
                || path.arrival > cache.windows[path.group as usize]
                || path.leave > path.arrival
            {
                return Err(invalid);
            }
            for i in 0..3 {
                let leg = take(8)?;
                path.legs[i] = Leg {
                    line: leg[0],
                    from: leg[1],
                    to: leg[2],
                    flags: leg[3],
                    departure: u16::from_le_bytes(leg[4..6].try_into().unwrap()),
                    arrival: u16::from_le_bytes(leg[6..8].try_into().unwrap()),
                };
                if leg[3] & !(LEG_REQUEST_BOARD | LEG_REQUEST_ALIGHT) != 0
                    || i >= path.len as usize && leg[3] != 0
                {
                    return Err(invalid);
                }
                if i < path.len as usize
                    && (labels.get(leg[0]).is_none()
                        || labels.get(leg[1]).is_none()
                        || labels.get(leg[2]).is_none()
                        || path.legs[i].departure > path.legs[i].arrival)
                {
                    return Err(invalid);
                }
            }
            if u32::from(path.leave) + u32::from(path.walk) * 60
                != u32::from(path.legs[0].departure)
                || u32::from(path.arrival)
                    != u32::from(path.legs[path.len as usize - 1].arrival)
                        + u32::from(path.egress) * 60
            {
                return Err(invalid);
            }
            for pair in path.legs[..path.len as usize].windows(2) {
                if u32::from(pair[0].arrival) + u32::from(journey.transfer_minutes) * 60
                    > u32::from(pair[1].departure)
                {
                    return Err(invalid);
                }
            }
            cache.paths.push(path).map_err(|_| invalid)?;
        }
        if at != input.len() {
            return Err(invalid);
        }
        Ok(cache)
    }
}

/// Trips, routes and services are identified by a 64-bit FNV-1a key instead of
/// their ID text. GTFS IDs are unique in trips.txt, routes.txt and calendar.txt,
/// so a colliding stored key appears as a duplicate there and fails visibly.
fn key(bytes: &[u8]) -> [u32; 2] {
    fold(0xcbf2_9ce4_8422_2325, bytes)
}
fn fold(mut hash: u64, bytes: &[u8]) -> [u32; 2] {
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    [hash as u32, (hash >> 32) as u32]
}

#[derive(Clone, Copy)]
struct Trip {
    key: [u32; 2],
    kind: u8,
    active: u8,
    route: u8,
    seen: bool,
}
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub(crate) struct Call {
    sequence: u32,
    trip: u16,
    stop: u16,
    arrival: u16,
    departure: u16,
    day: u8,
    flags: u8,
}
#[derive(Clone, Copy)]
struct Ride {
    trip: u16,
    from: u16,
    to: u16,
    departure: u16,
    arrival: u16,
    endpoint: u8,
    day: u8,
}
#[derive(Clone, Copy)]
struct Rule {
    from: u16,
    to: u16,
    seconds: u16,
    forbidden: bool,
}
struct Service {
    key: [u32; 2],
    active: u8,
    removed: u8,
}
#[derive(Clone, Copy)]
struct Candidate {
    first: Ride,
    middle: Option<Ride>,
    last: Option<Ride>,
    target: u8,
}
impl Candidate {
    fn end(self) -> Ride {
        self.last.or(self.middle).unwrap_or(self.first)
    }
    fn len(self) -> u8 {
        1 + u8::from(self.middle.is_some()) + u8::from(self.last.is_some())
    }
}
/// Fixed-size place identity: a declared station, a stop ID, or a normalized
/// name at a quantized position. Names compare by key; positions exactly.
#[derive(Clone, Copy, PartialEq, Eq)]
struct PlaceKey {
    tag: u8,
    hash: [u32; 2],
    lat: i32,
    lon: i32,
}
const NO_PLACE: PlaceKey = PlaceKey {
    tag: 0,
    hash: [0; 2],
    lat: 0,
    lon: 0,
};

/// Four Bloom sets of two probes each: stops and places reached after boarding
/// at a start (side 0), and stops and places where a ride towards a target can
/// be boarded (side 1). False positives only keep extra candidate stops.
const FILTER: usize = 2560;
const SET_BITS: usize = FILTER * 2;
const STOP_SIDES: usize = 0;
const PLACE_SIDES: usize = 2;
// FNV-1a mixes low bits weakly for short numeric IDs; finish with fmix64.
fn probes(set: usize, value: [u32; 2]) -> [usize; 2] {
    let mut h = u64::from(value[0]) | (u64::from(value[1]) << 32);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    [h as u32, (h >> 32) as u32].map(|half| set * SET_BITS + half as usize % SET_BITS)
}
fn contains(filter: &[u8; FILTER], set: usize, value: [u32; 2]) -> bool {
    probes(set, value)
        .iter()
        .all(|&bit| filter[bit / 8] & (1 << (bit % 8)) != 0)
}
fn insert(filter: &mut [u8; FILTER], set: usize, value: [u32; 2]) {
    for bit in probes(set, value) {
        filter[bit / 8] |= 1 << (bit % 8);
    }
}
fn sides(filter: &[u8; FILTER], base: usize, value: [u32; 2]) -> u8 {
    u8::from(contains(filter, base, value)) | (u8::from(contains(filter, base + 1, value)) * 2)
}
fn insert_sides(filter: &mut [u8; FILTER], base: usize, value: [u32; 2], bits: u8) {
    for side in 0..2 {
        if bits & (1 << side) != 0 {
            insert(filter, base + side, value);
        }
    }
}

/// One region holds whichever transient index the current phase needs.
#[allow(clippy::large_enum_variant)] // Deliberate allocation-free reuse.
enum Scratch<const STOPS: usize, const MIDDLE: usize> {
    Idle,
    Keys(Vec<PlaceKey, STOPS>),
    Filter([u8; FILTER]),
    Middle(Vec<Call, MIDDLE>),
}
impl<const STOPS: usize, const MIDDLE: usize> Scratch<STOPS, MIDDLE> {
    fn keys(&mut self) -> &mut Vec<PlaceKey, STOPS> {
        if !matches!(self, Self::Keys(_)) {
            *self = Self::Keys(Vec::new());
        }
        match self {
            Self::Keys(keys) => keys,
            _ => unreachable!(),
        }
    }
    fn filter(&mut self) -> &mut [u8; FILTER] {
        if !matches!(self, Self::Filter(_)) {
            *self = Self::Filter([0; FILTER]);
        }
        match self {
            Self::Filter(filter) => filter,
            _ => unreachable!(),
        }
    }
    fn middle(&mut self) -> &mut Vec<Call, MIDDLE> {
        if !matches!(self, Self::Middle(_)) {
            *self = Self::Middle(Vec::new());
        }
        match self {
            Self::Middle(calls) => calls,
            _ => unreachable!(),
        }
    }
}

/// All transient indexes are caller-owned, allocation-free and reused per group.
pub struct Search<
    const CALLS: usize,
    const STOPS: usize,
    const TRIPS: usize,
    const MIDDLE: usize,
    const SERVICES: usize,
    const ROUTES: usize,
> {
    trips: Vec<Trip, TRIPS>,
    /// Trip indexes sorted by key.
    lookup: Vec<u16, TRIPS>,
    stops: Vec<Id<32>, STOPS>,
    stop_lookup: Vec<u16, STOPS>,
    parents: [u16; STOPS],
    places: [u16; STOPS],
    routes: Vec<[u32; 2], ROUTES>,
    scratch: Scratch<STOPS, MIDDLE>,
    rules: Vec<Rule, 128>,
    services: Vec<Service, SERVICES>,
    candidates: Vec<Candidate, MAX_PATHS>,
    dates: [Date; 3],
    epochs: [i64; 3],
    now: i64,
    /// Seconds after `now` that the profile covers; shrinks only to fit.
    horizon: u16,
    limits: u8,
    /// Calls before this index are at chosen places, sorted by trip instance.
    endpoint_len: usize,
    manifest: Option<zip::Manifest>,
    // One bit per origin (low nibble) and target (high nibble). Platforms
    // share an endpoint without consuming additional configured-stop slots.
    endpoints: [u8; STOPS],
}
pub type Planner = Search<ANDROID_CALLS, 512, 1536, 1024, 256, 256>;
pub type DevicePlanner = Search<DEVICE_CALLS, 128, 512, 128, 64, 32>;
impl<
    const CALLS: usize,
    const STOPS: usize,
    const TRIPS: usize,
    const MIDDLE: usize,
    const SERVICES: usize,
    const ROUTES: usize,
> Search<CALLS, STOPS, TRIPS, MIDDLE, SERVICES, ROUTES>
{
    const STOPS_FIT: () = assert!(STOPS <= 1024 && TRIPS < NONE as usize);
    pub fn new(today: Date) -> Self {
        let () = Self::STOPS_FIT;
        Self {
            trips: Vec::new(),
            lookup: Vec::new(),
            stops: Vec::new(),
            stop_lookup: Vec::new(),
            parents: [NONE; STOPS],
            places: [NONE; STOPS],
            routes: Vec::new(),
            scratch: Scratch::Idle,
            rules: Vec::new(),
            services: Vec::new(),
            candidates: Vec::new(),
            dates: [today; 3],
            epochs: [0; 3],
            now: 0,
            horizon: HORIZON as u16,
            limits: 0,
            endpoint_len: 0,
            manifest: None,
            endpoints: [0; STOPS],
        }
    }
    fn find_trip(&self, id: &str) -> Option<usize> {
        let k = key(id.as_bytes());
        self.lookup
            .binary_search_by(|&i| self.trips[i as usize].key.cmp(&k))
            .ok()
            .map(|i| self.lookup[i] as usize)
    }
    /// Returns None when the trip table is full.
    fn intern(&mut self, id: &str, kind: u8) -> Result<Option<u16>, Error> {
        if id.is_empty() {
            return Err(Error::InvalidField("trip_id"));
        }
        let k = key(id.as_bytes());
        match self
            .lookup
            .binary_search_by(|&i| self.trips[i as usize].key.cmp(&k))
        {
            Ok(at) => {
                let i = self.lookup[at];
                self.trips[i as usize].kind |= kind;
                Ok(Some(i))
            }
            Err(at) => {
                let i = self.trips.len() as u16;
                let trip = Trip {
                    key: k,
                    kind,
                    active: 0,
                    route: 0,
                    seen: false,
                };
                if self.trips.push(trip).is_err() {
                    return Ok(None);
                }
                self.lookup.insert(at, i).map_err(|_| Error::IndexFull)?;
                Ok(Some(i))
            }
        }
    }
    fn index_trips(&mut self) {
        self.lookup.clear();
        for i in 0..self.trips.len() {
            self.lookup.push(i as u16).unwrap();
        }
        let trips = &self.trips;
        self.lookup.sort_unstable_by_key(|&i| trips[i as usize].key);
    }
    fn index_stops(&mut self) {
        self.stop_lookup.clear();
        for i in 0..self.stops.len() {
            self.stop_lookup.push(i as u16).unwrap();
        }
        let stops = &self.stops;
        self.stop_lookup
            .sort_unstable_by(|&a, &b| stops[a as usize].cmp(&stops[b as usize]));
    }
    fn indexed_stop(&self, id: &str) -> Option<u16> {
        self.stop_lookup
            .binary_search_by(|&i| self.stops[i as usize].as_str().cmp(id))
            .ok()
            .map(|i| self.stop_lookup[i])
    }
    fn stop_index(&self, id: &str) -> Option<u16> {
        self.stops
            .iter()
            .position(|s| s.as_str() == id)
            .map(|i| i as u16)
    }
    fn stop(&mut self, id: &str) -> Result<u16, Error> {
        if let Some(i) = self.stop_index(id) {
            return Ok(i);
        }
        self.stops
            .push(Id::new(id)?)
            .map_err(|_| Error::IndexFull)?;
        Ok((self.stops.len() - 1) as u16)
    }
    fn truncate_stops(&mut self, len: usize) {
        self.stops.truncate(len);
        self.parents[len..].fill(NONE);
        self.places[len..].fill(NONE);
        self.endpoints[len..].fill(0);
    }
    fn route(&mut self, id: &str) -> Result<u8, Error> {
        let k = key(id.as_bytes());
        if let Some(i) = self.routes.iter().position(|&r| r == k) {
            return Ok(i as u8);
        }
        self.routes.push(k).map_err(|_| Error::IndexFull)?;
        Ok((self.routes.len() - 1) as u8)
    }
    fn service(&mut self, id: &str) -> Result<&mut Service, Error> {
        let k = key(id.as_bytes());
        let i = if let Some(i) = self.services.iter().position(|s| s.key == k) {
            i
        } else {
            self.services
                .push(Service {
                    key: k,
                    active: 0,
                    removed: 0,
                })
                .map_err(|_| Error::IndexFull)?;
            self.services.len() - 1
        };
        Ok(&mut self.services[i])
    }
    fn offset(&self, time: Option<gtfs::Time>, day: usize) -> Option<u16> {
        let at = self.epochs[day].checked_add(i64::from(time?.seconds()))? - self.now;
        (0..=i64::from(self.horizon))
            .contains(&at)
            .then_some(at as u16)
    }
    /// In-window times and boarding flags for one service day. A dwell can
    /// straddle either boundary; only the in-window timestamp grants boarding
    /// or alighting, and clamping never invents it.
    fn window(&self, call: &StopTime<'_>, day: usize) -> Result<Option<(u16, u16, u8)>, Error> {
        let (Some(arrival), Some(departure)) = (call.arrival, call.departure) else {
            return Ok(None);
        };
        let arrival = self.epochs[day] + i64::from(arrival.seconds()) - self.now;
        let departure = self.epochs[day] + i64::from(departure.seconds()) - self.now;
        if arrival > departure {
            return Err(Error::InvalidTime);
        }
        let horizon = i64::from(self.horizon);
        if departure < 0 || arrival > horizon {
            return Ok(None);
        }
        let board = (0..=horizon).contains(&departure);
        let alight = (0..=horizon).contains(&arrival);
        let flags = u8::from(board && call.pickup_type != 1)
            | (u8::from(alight && call.drop_off_type != 1) * 2)
            | (u8::from(call.pickup_type >= 2) * 4)
            | (u8::from(call.drop_off_type >= 2) * 8);
        Ok(Some((
            arrival.clamp(0, horizon) as u16,
            departure.clamp(0, horizon) as u16,
            flags,
        )))
    }
    /// Stores one in-window call, ending the window earlier when full.
    #[allow(clippy::too_many_arguments)]
    fn push<const N: usize>(
        &mut self,
        calls: &mut Vec<Call, N>,
        call: &StopTime<'_>,
        trip: u16,
        stop: u16,
        day: usize,
    ) -> Result<(), Error> {
        loop {
            let Some((arrival, departure, flags)) = self.window(call, day)? else {
                return Ok(());
            };
            let value = Call {
                sequence: call.sequence,
                trip,
                stop,
                arrival,
                departure,
                day: day as u8,
                flags,
            };
            if calls.push(value).is_ok() {
                return Ok(());
            }
            self.shrink(calls, false)?;
        }
    }
    /// End the window a quarter earlier. Every connection that arrives by the
    /// new end uses only retained calls, so the shorter profile stays exact.
    fn shrink<const N: usize>(
        &mut self,
        calls: &mut Vec<Call, N>,
        trips: bool,
    ) -> Result<(), Error> {
        let horizon = self.horizon - self.horizon / 4;
        if horizon < MIN_WINDOW {
            return Err(Error::IndexFull);
        }
        self.horizon = horizon;
        self.limits |= LIMIT_WINDOW;
        let endpoint_len = self.endpoint_len;
        let mut index = 0;
        let mut kept = 0;
        calls.retain_mut(|c| {
            let keep = c.arrival <= horizon;
            if keep && c.departure > horizon {
                c.departure = horizon;
                c.flags &= !1;
            }
            kept += usize::from(keep && index < endpoint_len);
            index += 1;
            keep
        });
        self.endpoint_len = kept;
        self.candidates
            .retain(|c| u32::from(c.end().arrival) <= u32::from(horizon));
        if trips {
            self.compact_trips(calls);
        }
        Ok(())
    }
    /// Drop trips without retained calls, renumbering calls in place.
    fn compact_trips<const N: usize>(&mut self, calls: &mut Vec<Call, N>) {
        for t in &mut self.trips {
            t.seen = false;
        }
        for c in calls.iter() {
            self.trips[c.trip as usize].seen = true;
        }
        self.lookup.clear();
        let mut next = 0;
        for i in 0..self.trips.len() {
            if self.trips[i].seen {
                self.trips[next] = self.trips[i];
                self.trips[next].seen = false;
                self.lookup.push(next as u16).unwrap();
                next += 1;
            } else {
                self.lookup.push(NONE).unwrap();
            }
        }
        for c in calls.iter_mut() {
            c.trip = self.lookup[c.trip as usize];
        }
        self.trips.truncate(next);
        self.index_trips();
    }
    // The user's minimum applies even to GTFS timed transfers. A guarantee
    // never grants permission to waive their chosen change-time margin.
    fn transfer(&self, from: u16, to: u16, slack: u8) -> Option<u16> {
        let from_parent = self.parents[from as usize];
        let to_parent = self.parents[to as usize];
        let rule = self
            .rules
            .iter()
            .filter(|r| {
                (r.from == from || r.from == from_parent) && (r.to == to || r.to == to_parent)
            })
            .max_by_key(|r| u8::from(r.from == from) + u8::from(r.to == to));
        if let Some(rule) = rule {
            return (!rule.forbidden).then_some(rule.seconds.max(u16::from(slack) * 60));
        }
        (from == to
            || from_parent != NONE && from_parent == to_parent
            || self.places[from as usize] != NONE
                && self.places[from as usize] == self.places[to as usize])
            .then_some(u16::from(slack) * 60)
    }
    async fn stations(
        &mut self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
        expand: bool,
    ) -> Result<(), Error> {
        let manifest = self.manifest;
        let before = self.stops.len();
        source
            .scan(&[Table::Stops], manifest.as_ref(), workspace, |row| {
                let stop = row.stop()?;
                if let Some(i) = self.stop_index(stop.id).filter(|&i| (i as usize) < before)
                    && let Some(parent) = stop.parent_station
                {
                    self.parents[i as usize] = self.stop(parent)?;
                }
                Ok(())
            })
            .await?;
        let mut seen = [0u8; 128];
        source
            .scan(&[Table::Stops], manifest.as_ref(), workspace, |row| {
                let stop = row.stop()?;
                if let Some(i) = self.stop_index(stop.id)
                    && stop.location_type == 1
                {
                    *seen.get_mut(i as usize / 8).ok_or(Error::IndexFull)? |= 1 << (i as usize % 8);
                }
                if expand
                    && stop.location_type == 0
                    && let Some(parent) = stop.parent_station.and_then(|p| self.stop_index(p))
                    && (self.parents[..before].contains(&parent)
                        || self
                            .rules
                            .iter()
                            .any(|r| r.from == parent || r.to == parent))
                {
                    let child = self.stop(stop.id)?;
                    self.parents[child as usize] = parent;
                }
                Ok(())
            })
            .await?;
        if self.parents[..self.stops.len()].iter().any(|&p| {
            p != NONE
                && seen
                    .get(p as usize / 8)
                    .is_none_or(|byte| *byte & (1 << (p as usize % 8)) == 0)
        }) {
            return Err(Error::InvalidField("unknown parent_station"));
        }
        Ok(())
    }
    async fn group_places(
        &mut self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        let manifest = self.manifest;
        let keys = self.scratch.keys();
        keys.clear();
        for _ in 0..self.stops.len() {
            keys.push(NO_PLACE).map_err(|_| Error::IndexFull)?;
        }
        // Collect the whole relevant name/station cohort, including distant
        // homonyms, before partitioning: choosing only a selected anchor's
        // halo would give different groups.
        source
            .scan(&[Table::Stops], manifest.as_ref(), workspace, |row| {
                let stop = row.stop()?;
                if stop.location_type == 0
                    && let Some(i) = self.stop_index(stop.id)
                {
                    self.scratch.keys()[i as usize] = place_key(stop)?;
                }
                Ok(())
            })
            .await?;
        source
            .scan(&[Table::Stops], manifest.as_ref(), workspace, |row| {
                let stop = row.stop()?;
                if stop.location_type != 0 {
                    return Ok(());
                }
                if self
                    .stop_index(stop.id)
                    .is_some_and(|i| self.scratch.keys()[i as usize] != NO_PLACE)
                {
                    return Ok(());
                }
                let cohort = Cohort::of(stop);
                if self.scratch.keys().iter().any(|&k| cohort.contains(k)) {
                    let i = self.stop(stop.id)?;
                    let key = place_key(stop)?;
                    let keys = self.scratch.keys();
                    while keys.len() < i as usize + 1 {
                        keys.push(NO_PLACE).map_err(|_| Error::IndexFull)?;
                    }
                    keys[i as usize] = key;
                }
                Ok(())
            })
            .await?;
        self.places.fill(NONE);
        let Scratch::Keys(keys) = &self.scratch else {
            unreachable!()
        };
        // Byte-order stop IDs define the same greedy representatives on every
        // platform, independently of file order and which endpoints were chosen.
        while let Some(i) = (0..self.stops.len())
            .filter(|&i| keys[i] != NO_PLACE && self.places[i] == NONE)
            .min_by(|&a, &b| self.stops[a].cmp(&self.stops[b]))
        {
            let representative = (0..self.stops.len())
                .filter(|&j| self.places[j] == j as u16 && same_place_keys(keys[i], keys[j]))
                .min_by(|&a, &b| self.stops[a].cmp(&self.stops[b]));
            self.places[i] = representative.unwrap_or(i) as u16;
        }
        self.scratch = Scratch::Idle;
        Ok(())
    }
    fn join(&self, a: Ride, b: Ride, slack: u8) -> bool {
        (a.trip != b.trip || a.day & DAY != b.day & DAY)
            && self.transfer(a.to, b.from, slack).is_some_and(|seconds| {
                u32::from(a.arrival) + u32::from(seconds) <= u32::from(b.departure)
            })
    }
    /// Keeps nondominated connections. Any configured target counts as arrival:
    /// a later departure that arrives no later is better, whatever the target.
    fn candidate(
        candidates: &mut Vec<Candidate, MAX_PATHS>,
        horizon: &mut u16,
        limits: &mut u8,
        c: Candidate,
        config: &Journey,
    ) -> Result<(), Error> {
        let score = |c: Candidate| {
            let walk = u16::from(config.origins[c.first.endpoint as usize].walk) * 60;
            let egress = u32::from(config.targets[c.target as usize].walk) * 60;
            (
                c.first.departure.checked_sub(walk),
                u32::from(c.end().arrival) + egress,
                (c.len(), walk),
            )
        };
        let (Some(leave), arrive, rank) = score(c) else {
            return Ok(());
        };
        if arrive > u32::from(*horizon) {
            return Ok(());
        }
        let dominates = |a: Candidate, (leave, arrive, rank): (u16, u32, (u8, u16))| {
            let (a_leave, a_arrive, a_rank) = score(a);
            a_leave.is_some_and(|l| l >= leave)
                && a_arrive <= arrive
                && (a_arrive < arrive || a_rank <= rank)
        };
        if candidates
            .iter()
            .any(|&old| dominates(old, (leave, arrive, rank)))
        {
            return Ok(());
        }
        candidates.retain(|&old| {
            let (l, a, r) = score(old);
            !dominates(c, (l.unwrap_or(0), a, r))
        });
        while candidates.is_full() {
            // Keep the earliest arrivals; the profile ends before the latest.
            let latest = candidates
                .iter()
                .map(|&old| score(old).1)
                .chain([arrive])
                .max()
                .unwrap();
            let end = u16::try_from(latest.saturating_sub(1)).unwrap_or(u16::MAX);
            if end < MIN_WINDOW {
                return Err(Error::IndexFull);
            }
            *horizon = end;
            *limits |= LIMIT_WINDOW;
            candidates.retain(|&old| score(old).1 <= u32::from(end));
            if arrive > u32::from(end) {
                return Ok(());
            }
        }
        candidates.push(c).map_err(|_| Error::IndexFull)
    }
    fn endpoint_paths(
        &mut self,
        calls: &mut [Call],
        config: &Journey,
        depth: u8,
    ) -> Result<(), Error> {
        calls.sort_unstable_by_key(|c| (c.trip, c.day, c.sequence));
        for pair in calls.windows(2) {
            if pair[0].trip == pair[1].trip
                && pair[0].day == pair[1].day
                && (pair[0].sequence == pair[1].sequence || pair[0].departure > pair[1].arrival)
            {
                return Err(Error::InvalidField("stop_sequence or stop times"));
            }
        }
        for first in first_rides(calls, &self.endpoints, config) {
            for target in 0..config.targets.len() {
                if self.endpoints[first.to as usize] & (1 << (target + 4)) != 0 {
                    Self::candidate(
                        &mut self.candidates,
                        &mut self.horizon,
                        &mut self.limits,
                        Candidate {
                            first,
                            middle: None,
                            last: None,
                            target: target as u8,
                        },
                        config,
                    )?;
                }
            }
            if depth != 0 {
                for last in last_rides(calls, &self.endpoints, config) {
                    if self.join(first, last, config.transfer_minutes) {
                        Self::candidate(
                            &mut self.candidates,
                            &mut self.horizon,
                            &mut self.limits,
                            Candidate {
                                first,
                                middle: None,
                                last: Some(last),
                                target: last.endpoint,
                            },
                            config,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
    async fn metadata(
        &mut self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        for t in &mut self.trips {
            t.seen = false;
            t.active = 0;
        }
        let manifest = self.manifest;
        source
            .scan(&[Table::Trips], manifest.as_ref(), workspace, |row| {
                let id = row.required(C::TripId)?;
                if let Some(i) = self.find_trip(id) {
                    if self.trips[i].seen {
                        return Err(Error::DuplicateTrip);
                    }
                    let trip = row.trip()?;
                    let service = key(trip.service_id.as_bytes());
                    let active = self
                        .services
                        .iter()
                        .find(|s| s.key == service)
                        .map_or(0, |s| s.active);
                    let route = self.route(trip.route_id)?;
                    self.trips[i].active = active;
                    self.trips[i].route = route;
                    self.trips[i].seen = true;
                }
                Ok(())
            })
            .await?;
        if self.trips.iter().any(|t| !t.seen) {
            return Err(Error::InvalidField("unknown trip_id"));
        }
        Ok(())
    }
    /// Pass 1: every in-window call at a chosen place. Service days are not
    /// known yet, so inactive trips are dropped after the trips table.
    async fn endpoint_calls<const N: usize>(
        &mut self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
        mask: u8,
        calls: &mut Vec<Call, N>,
    ) -> Result<(), Error> {
        self.index_stops();
        let manifest = self.manifest;
        source
            .scan(&[Table::StopTimes], manifest.as_ref(), workspace, |row| {
                let Some(stop) = self.indexed_stop(row.required(C::StopId)?) else {
                    return Ok(());
                };
                let members = self.endpoints[stop as usize];
                if members == 0 {
                    return Ok(());
                }
                let call = row.stop_time()?;
                for day in 0..3 {
                    if mask & (1 << day) == 0 {
                        continue;
                    }
                    let Some((_, _, flags)) = self.window(&call, day)? else {
                        continue;
                    };
                    let kind = u8::from(flags & 1 != 0 && members & 15 != 0)
                        | (u8::from(flags & 2 != 0 && members & 240 != 0) * 2);
                    let trip = loop {
                        if let Some(trip) = self.intern(call.trip_id, kind)? {
                            break trip;
                        }
                        self.shrink(calls, true)?;
                    };
                    self.push(calls, &call, trip, stop, day)?;
                }
                Ok(())
            })
            .await?;
        self.endpoint_len = calls.len();
        Ok(())
    }
    /// Keeps active endpoint calls of trips that can start or finish a ride.
    fn keep_active<const N: usize>(&mut self, calls: &mut Vec<Call, N>) {
        calls.retain(|c| self.trips[c.trip as usize].active & (1 << c.day) != 0);
        for t in &mut self.trips {
            t.kind = 0;
        }
        for c in calls.iter() {
            let members = self.endpoints[c.stop as usize];
            self.trips[c.trip as usize].kind |= u8::from(c.flags & 1 != 0 && members & 15 != 0)
                | (u8::from(c.flags & 2 != 0 && members & 240 != 0) * 2);
        }
        calls.retain(|c| self.trips[c.trip as usize].kind != 0);
        calls.sort_unstable_by_key(|c| (c.trip, c.day, c.sequence));
        self.endpoint_len = calls.len();
    }
    /// Rides of one trip instance may continue after its first boarding at a
    /// start and lead to its last arrival at a target.
    fn bounds(&self, endpoint: &[Call], trip: u16, day: u8) -> (Option<u32>, Option<u32>) {
        let start = endpoint.partition_point(|c| (c.trip, c.day) < (trip, day));
        let (mut board, mut alight) = (None::<u32>, None::<u32>);
        for c in endpoint[start..]
            .iter()
            .take_while(|c| (c.trip, c.day) == (trip, day))
        {
            let (sequence, members) = (c.sequence, self.endpoints[c.stop as usize]);
            if c.flags & 1 != 0 && members & 15 != 0 {
                board = Some(board.map_or(sequence, |b| b.min(sequence)));
            }
            if c.flags & 2 != 0 && members & 240 != 0 {
                alight = Some(alight.map_or(sequence, |a| a.max(sequence)));
            }
        }
        (board, alight)
    }
    /// Pass 2 and two stop scans: find places that a ride from a start reaches
    /// and a ride towards a target serves, then index only their stops.
    async fn transfer_stops(
        &mut self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
        mask: u8,
        calls: &[Call],
        depth: u8,
    ) -> Result<u8, Error> {
        let manifest = self.manifest;
        let endpoint = &calls[..self.endpoint_len];
        self.scratch.filter();
        source
            .scan(&[Table::StopTimes], manifest.as_ref(), workspace, |row| {
                let Some(trip) = self.find_trip(row.required(C::TripId)?) else {
                    return Ok(());
                };
                let call = row.stop_time()?;
                let mut bits = 0;
                for day in 0..3u8 {
                    if mask & self.trips[trip].active & (1 << day) == 0 {
                        continue;
                    }
                    let Some((_, _, flags)) = self.window(&call, day as usize)? else {
                        continue;
                    };
                    let (board, alight) = self.bounds(endpoint, trip as u16, day);
                    bits |= u8::from(flags & 2 != 0 && board.is_some_and(|b| call.sequence > b))
                        | (u8::from(flags & 1 != 0 && alight.is_some_and(|a| call.sequence < a))
                            * 2);
                }
                if bits != 0 {
                    let stop = key(call.stop_id.as_bytes());
                    insert_sides(self.scratch.filter(), STOP_SIDES, stop, bits);
                }
                Ok(())
            })
            .await?;
        source
            .scan(&[Table::Stops], manifest.as_ref(), workspace, |row| {
                let stop = row.stop()?;
                let filter = self.scratch.filter();
                let bits = sides(filter, STOP_SIDES, key(stop.id.as_bytes()));
                if stop.location_type == 0 && bits != 0 {
                    insert_sides(filter, PLACE_SIDES, place_filter_key(stop), bits);
                }
                Ok(())
            })
            .await?;
        // Explicit transfer links may join different places; keep their stops.
        let filter = self.scratch.filter();
        filter[..FILTER / 2].fill(0);
        // The stop sets are no longer needed; set 0 now holds linked stops.
        if manifest
            .as_ref()
            .is_some_and(|m| m.get(Table::Transfers).is_some())
        {
            source
                .scan(&[Table::Transfers], manifest.as_ref(), workspace, |row| {
                    for column in [C::FromStopId, C::ToStopId] {
                        let id = row.required(column)?;
                        insert(self.scratch.filter(), STOP_SIDES, key(id.as_bytes()));
                    }
                    Ok(())
                })
                .await?;
        }
        let base = self.stops.len();
        let mut depth = depth;
        loop {
            let both = depth < 2;
            let result = source
                .scan(&[Table::Stops], manifest.as_ref(), workspace, |row| {
                    let stop = row.stop()?;
                    if stop.location_type != 0 || self.stop_index(stop.id).is_some() {
                        return Ok(());
                    }
                    let filter = self.scratch.filter();
                    let place = sides(filter, PLACE_SIDES, place_filter_key(stop));
                    let linked = contains(filter, STOP_SIDES, key(stop.id.as_bytes()))
                        || stop
                            .parent_station
                            .is_some_and(|p| contains(filter, STOP_SIDES, key(p.as_bytes())));
                    if linked || if both { place == 3 } else { place != 0 } {
                        self.stop(stop.id)?;
                    }
                    Ok(())
                })
                .await;
            match result {
                Ok(_) => break,
                // Two transfers need every reachable change place. When they do
                // not fit, search one transfer and say so.
                Err(Error::IndexFull) if depth == 2 => {
                    self.truncate_stops(base);
                    self.limits |= LIMIT_TRANSFERS;
                    depth = 1;
                }
                Err(error) => return Err(error),
            }
        }
        self.scratch = Scratch::Idle;
        Ok(depth)
    }
    /// Pass 3: calls at the indexed change places, within each trip instance's
    /// useful sequence range.
    async fn transfer_calls<const N: usize>(
        &mut self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
        mask: u8,
        calls: &mut Vec<Call, N>,
    ) -> Result<(), Error> {
        let manifest = self.manifest;
        self.index_stops();
        source
            .scan(&[Table::StopTimes], manifest.as_ref(), workspace, |row| {
                let Some(trip) = self.find_trip(row.required(C::TripId)?) else {
                    return Ok(());
                };
                let Some(stop) = self.indexed_stop(row.required(C::StopId)?) else {
                    return Ok(());
                };
                // Calls at chosen places were all kept in the first pass.
                if self.endpoints[stop as usize] != 0 {
                    return Ok(());
                }
                let call = row.stop_time()?;
                for day in 0..3u8 {
                    if mask & self.trips[trip].active & (1 << day) == 0 {
                        continue;
                    }
                    let (board, alight) =
                        self.bounds(&calls[..self.endpoint_len], trip as u16, day);
                    if board.is_some_and(|b| call.sequence > b)
                        || alight.is_some_and(|a| call.sequence < a)
                    {
                        self.push(calls, &call, trip as u16, stop, day as usize)?;
                    }
                }
                Ok(())
            })
            .await?;
        Ok(())
    }
    /// Change places, station links and transfer rules for one-transfer search.
    async fn transfers<const N: usize>(
        &mut self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
        mask: u8,
        calls: &mut Vec<Call, N>,
        depth: u8,
    ) -> Result<u8, Error> {
        let depth = self
            .transfer_stops(source, workspace, mask, calls, depth)
            .await?;
        self.transfer_calls(source, workspace, mask, calls).await?;
        self.group_places(source, workspace).await?;
        self.stations(source, workspace, false).await?;
        // The idle stop lookup marks actual endpoint-trip change anchors.
        // Distant homonyms indexed for partitioning are not transfer anchors.
        self.stop_lookup.clear();
        for _ in 0..self.stops.len() {
            self.stop_lookup.push(0).map_err(|_| Error::IndexFull)?;
        }
        for call in calls.iter() {
            self.stop_lookup[call.stop as usize] = 1;
            let parent = self.parents[call.stop as usize];
            if parent != NONE {
                self.stop_lookup[parent as usize] = 1;
            }
        }
        // Copy the manifest only around this scan: child futures above keep
        // their own copies and should not share the async state with this one.
        let manifest = self.manifest;
        source
            .scan(&[Table::Transfers], manifest.as_ref(), workspace, |row| {
                let from = row.required(C::FromStopId)?;
                let to = row.required(C::ToStopId)?;
                if ![from, to].iter().any(|s| {
                    self.stop_index(s)
                        .is_some_and(|i| self.stop_lookup.get(i as usize) == Some(&1))
                }) {
                    return Ok(());
                }
                // This bounded mode admits stop-level rules only. Scoped
                // rules touching a selected trip's stops fail explicitly;
                // their route/trip predicates are not approximated.
                if [C::FromRouteId, C::ToRouteId, C::FromTripId, C::ToTripId]
                    .iter()
                    .any(|&c| row.get(c).is_some())
                {
                    return Err(Error::InvalidField(
                        "journey routing does not yet support route-specific transfers",
                    ));
                }
                let kind = row
                    .get(C::TransferType)
                    .unwrap_or("0")
                    .parse::<u8>()
                    .map_err(|_| Error::InvalidField("transfer_type"))?;
                if kind > 3 {
                    return Err(Error::InvalidField(
                        "journey routing does not yet support in-seat transfer rules",
                    ));
                }
                let seconds = row
                    .get(C::MinTransferTime)
                    .unwrap_or("0")
                    .parse::<u16>()
                    .map_err(|_| Error::InvalidField("min_transfer_time"))?;
                if kind == 2 && row.get(C::MinTransferTime).is_none() {
                    return Err(Error::MissingColumn("min_transfer_time"));
                }
                let from = self.stop(from)?;
                let to = self.stop(to)?;
                if self.rules.iter().any(|r| r.from == from && r.to == to) {
                    return Err(Error::InvalidField("duplicate transfer rule"));
                }
                self.rules
                    .push(Rule {
                        from,
                        to,
                        seconds,
                        forbidden: kind == 3,
                    })
                    .map_err(|_| Error::IndexFull)?;
                Ok(())
            })
            .await?;
        self.group_places(source, workspace).await?;
        Ok(depth)
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn query(
        &mut self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
        config: &Journey,
        zone: clock::Zone<'_>,
        now: i64,
        mask: u8,
        group: u8,
        output: &mut Cache,
        labels: &mut crate::model::Headings,
        expected: Option<&zip::Manifest>,
    ) -> Result<(), Error> {
        let mut calls = Vec::<Call, CALLS>::new();
        self.query_using(
            &mut calls, source, workspace, config, zone, now, mask, group, output, labels, expected,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)] // One bounded query with caller-owned source, workspace and output.
    pub(crate) async fn query_using<const N: usize>(
        &mut self,
        calls: &mut Vec<Call, N>,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
        config: &Journey,
        zone: clock::Zone<'_>,
        now: i64,
        mask: u8,
        group: u8,
        output: &mut Cache,
        labels: &mut crate::model::Headings,
        expected: Option<&zip::Manifest>,
    ) -> Result<(), Error> {
        config
            .validate()
            .map_err(|_| Error::InvalidField("journey settings"))?;
        self.trips.clear();
        self.lookup.clear();
        self.stops.clear();
        self.parents.fill(NONE);
        self.places.fill(NONE);
        self.routes.clear();
        calls.clear();
        self.scratch = Scratch::Idle;
        self.rules.clear();
        self.services.clear();
        self.candidates.clear();
        self.manifest = None;
        self.now = now;
        self.horizon = HORIZON as u16;
        self.limits = 0;
        self.endpoint_len = 0;
        let today = zone.service_date(now).ok_or(Error::InvalidDate)?;
        self.dates = [
            clock::previous(today).ok_or(Error::InvalidDate)?,
            today,
            clock::next(today).ok_or(Error::InvalidDate)?,
        ];
        for day in 0..3 {
            self.epochs[day] = zone
                .service_epoch(self.dates[day])
                .ok_or(Error::InvalidDate)?;
        }
        self.endpoints.fill(0);
        // The selected IDs seed a global, bounded place partition below.
        let mut found_origins = 0u8;
        let mut found_targets = 0u8;
        let manifest = source
            .scan(
                &[Table::Stops, Table::Calendar, Table::CalendarDates],
                expected,
                workspace,
                |row| {
                    if row.table() == Table::Stops {
                        let stop = row.stop()?;
                        for (i, s) in config.origins.iter().enumerate() {
                            if stop.id == s.stop {
                                if found_origins & (1 << i) != 0 || stop.location_type != 0 {
                                    return Err(Error::InvalidField(
                                        "duplicate or non-boarding journey stop",
                                    ));
                                }
                                self.stop(stop.id)?;
                                found_origins |= 1 << i;
                            }
                        }
                        for (i, s) in config.targets.iter().enumerate() {
                            if stop.id == s.stop {
                                if found_targets & (1 << i) != 0 || stop.location_type != 0 {
                                    return Err(Error::InvalidField(
                                        "duplicate or non-boarding journey stop",
                                    ));
                                }
                                self.stop(stop.id)?;
                                found_targets |= 1 << i;
                            }
                        }
                    } else {
                        for day in 0..3 {
                            if mask & (1 << day) == 0 {
                                continue;
                            }
                            if row.table() == Table::Calendar {
                                let calendar = row.calendar()?;
                                if calendar.runs_on(self.dates[day]) {
                                    let s = self.service(calendar.service_id)?;
                                    if s.removed & (1 << day) == 0 {
                                        s.active |= 1 << day;
                                    }
                                }
                            } else {
                                let exception = row.calendar_date()?;
                                if exception.date == self.dates[day] {
                                    let s = self.service(exception.service_id)?;
                                    if exception.added {
                                        s.active |= 1 << day;
                                    } else {
                                        s.active &= !(1 << day);
                                        s.removed |= 1 << day;
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                },
            )
            .await?;
        if found_origins != (1 << config.origins.len()) - 1
            || found_targets != (1 << config.targets.len()) - 1
        {
            return Err(Error::InvalidField("journey stop no longer exists"));
        }
        for t in [Table::Trips, Table::StopTimes] {
            if manifest.get(t).is_none() {
                return Err(Error::MissingTable(t));
            }
        }
        if manifest.get(Table::Calendar).is_none() && manifest.get(Table::CalendarDates).is_none() {
            return Err(Error::MissingTable(Table::Calendar));
        }
        self.manifest = Some(manifest);
        self.group_places(source, workspace).await?;
        for i in 0..self.stops.len() {
            for (bit, endpoint) in config
                .origins
                .iter()
                .enumerate()
                .chain(config.targets.iter().enumerate().map(|(i, s)| (i + 4, s)))
            {
                let anchor = self
                    .stop_index(&endpoint.stop)
                    .ok_or(Error::InvalidField("journey endpoint"))?;
                if self.places[i] == self.places[anchor as usize] {
                    self.endpoints[i] |= 1 << bit;
                }
            }
        }
        self.endpoint_calls(source, workspace, mask, calls).await?;
        self.metadata(source, workspace).await?;
        self.keep_active(calls);
        let base = self.stops.len();
        let mut depth = config.max_transfers;
        if depth != 0 {
            match self.transfers(source, workspace, mask, calls, depth).await {
                Ok(searched) => depth = searched,
                // Change places did not fit: keep the exact direct search.
                Err(Error::IndexFull) => {
                    calls.truncate(self.endpoint_len);
                    self.truncate_stops(base);
                    self.rules.clear();
                    self.scratch = Scratch::Idle;
                    self.limits |= LIMIT_TRANSFERS;
                    depth = 0;
                }
                Err(error) => return Err(error),
            }
        }
        self.endpoint_paths(calls, config, depth)?;
        if depth == 2 {
            // Earlier candidates stay valid; deeper search is reported as limited.
            match self.middle(source, workspace, mask, calls, config).await {
                Ok(()) => {}
                Err(Error::IndexFull) => {
                    self.scratch = Scratch::Idle;
                    self.limits |= LIMIT_TRANSFERS;
                }
                Err(error) => return Err(error),
            }
        }
        self.publish(source, workspace, config, group, output, labels)
            .await
    }
    /// Two transfers: a middle ride from a place a start reaches to a place a
    /// target ride serves.
    async fn middle<const N: usize>(
        &mut self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
        mask: u8,
        calls: &mut Vec<Call, N>,
        config: &Journey,
    ) -> Result<(), Error> {
        // Every change has one side on an already collected endpoint trip:
        // first.to -> middle.from, then middle.to -> last.from. Therefore
        // an applicable rule already touches the earlier endpoint stop set.
        self.stations(source, workspace, true).await?;
        self.group_places(source, workspace).await?;
        // Same-name expansion can introduce platforms in other declared
        // stations. Preserve their parent-specific transfer restrictions.
        self.stations(source, workspace, false).await?;
        if first_rides(calls, &self.endpoints, config).next().is_none()
            || last_rides(calls, &self.endpoints, config).next().is_none()
        {
            return Ok(());
        }
        self.index_stops();
        let manifest = self.manifest;
        // Per stop: the earliest time a first ride is ready to change there,
        // or UNREADY, plus FINISH when a last ride can be reached from it.
        // Both fit one u16 because ready times never exceed the window.
        let mut ready = [UNREADY; STOPS];
        for (stop, value) in ready.iter_mut().enumerate().take(self.stops.len()) {
            *value = first_rides(calls, &self.endpoints, config)
                .filter_map(|a| {
                    self.transfer(a.to, stop as u16, config.transfer_minutes)
                        .and_then(|s| a.arrival.checked_add(s))
                })
                .filter(|&at| at <= self.horizon)
                .min()
                .unwrap_or(UNREADY);
            if last_rides(calls, &self.endpoints, config).any(|l| {
                self.transfer(stop as u16, l.from, config.transfer_minutes)
                    .is_some()
            }) {
                *value |= FINISH;
            }
        }
        let boards = |stop: u16, departure: u16| {
            let at = ready[stop as usize] & !FINISH;
            at != UNREADY && at <= departure
        };
        source
            .scan(&[Table::StopTimes], manifest.as_ref(), workspace, |row| {
                let Some(stop) = self.indexed_stop(row.required(C::StopId)?) else {
                    return Ok(());
                };
                let call = row.stop_time()?;
                if call.pickup_type == 1 {
                    return Ok(());
                }
                if (0..3).any(|day| {
                    mask & (1 << day) != 0
                        && self
                            .offset(call.departure, day)
                            .is_some_and(|departure| boards(stop, departure))
                }) {
                    self.intern(call.trip_id, 4)?.ok_or(Error::IndexFull)?;
                }
                Ok(())
            })
            .await?;
        // Existing endpoint trips can also be the middle ride.
        self.metadata(source, workspace).await?;
        self.scratch.middle().clear();
        source
            .scan(&[Table::StopTimes], manifest.as_ref(), workspace, |row| {
                let Some(trip) = self
                    .find_trip(row.required(C::TripId)?)
                    .filter(|&i| self.trips[i].kind & 4 != 0)
                else {
                    return Ok(());
                };
                let Some(stop) = self.indexed_stop(row.required(C::StopId)?) else {
                    return Ok(());
                };
                let call = row.stop_time()?;
                for day in 0..3 {
                    if mask & self.trips[trip].active & (1 << day) == 0 {
                        continue;
                    }
                    let Some((arrival, departure, flags)) = self.window(&call, day)? else {
                        continue;
                    };
                    // Keep only calls that can start or end the middle ride.
                    if !(flags & 1 != 0 && boards(stop, departure)
                        || flags & 2 != 0 && ready[stop as usize] & FINISH != 0)
                    {
                        continue;
                    }
                    self.scratch
                        .middle()
                        .push(Call {
                            trip: trip as u16,
                            stop,
                            sequence: call.sequence,
                            arrival,
                            departure,
                            day: day as u8,
                            flags,
                        })
                        .map_err(|_| Error::IndexFull)?;
                }
                Ok(())
            })
            .await?;
        self.scratch
            .middle()
            .sort_unstable_by_key(|c| (c.trip, c.day, c.sequence));
        let Scratch::Middle(middle_calls) = &self.scratch else {
            unreachable!()
        };
        for i in 0..middle_calls.len() {
            let board = middle_calls[i];
            if board.flags & 1 == 0 {
                continue;
            }
            for j in i + 1..middle_calls.len() {
                let alight = middle_calls[j];
                if (board.trip, board.day) != (alight.trip, alight.day) {
                    break;
                }
                if board.sequence == alight.sequence {
                    return Err(Error::InvalidField("duplicate stop_sequence"));
                }
                if alight.flags & 2 == 0 || board.departure > alight.arrival {
                    continue;
                }
                let middle = Ride {
                    trip: board.trip,
                    from: board.stop,
                    to: alight.stop,
                    departure: board.departure,
                    arrival: alight.arrival,
                    endpoint: 0,
                    day: ride_day(board, alight),
                };
                for first in first_rides(calls, &self.endpoints, config) {
                    if !self.join(first, middle, config.transfer_minutes) {
                        continue;
                    }
                    for last in last_rides(calls, &self.endpoints, config) {
                        if self.join(middle, last, config.transfer_minutes) {
                            Self::candidate(
                                &mut self.candidates,
                                &mut self.horizon,
                                &mut self.limits,
                                Candidate {
                                    first,
                                    middle: Some(middle),
                                    last: Some(last),
                                    target: last.endpoint,
                                },
                                config,
                            )?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    async fn publish(
        &self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
        config: &Journey,
        group: u8,
        output: &mut Cache,
        labels: &mut crate::model::Headings,
    ) -> Result<(), Error> {
        let arrive = |c: &Candidate| {
            u32::from(c.end().arrival) + u32::from(config.targets[c.target as usize].walk) * 60
        };
        // Labels and result slots keep the earliest arrivals. Ending the
        // window before the first omitted arrival keeps the rest exact. The
        // ordering stays in this block, outside the async state of the scan.
        let (window, used_stops, used_routes) = {
            let mut order = Vec::<u8, MAX_PATHS>::new();
            for i in 0..self.candidates.len() {
                let _ = order.push(i as u8);
            }
            order.sort_unstable_by_key(|&i| arrive(&self.candidates[i as usize]));
            let room = MAX_PATHS - output.paths.len();
            let mut used_stops = Vec::<u16, 32>::new();
            let mut used_routes = Vec::<u8, 32>::new();
            let mut window = self.horizon;
            for (n, &i) in order.iter().enumerate() {
                let c = self.candidates[i as usize];
                let mut stops = used_stops.clone();
                let mut routes = used_routes.clone();
                let fits = n < room
                    && [Some(c.first), c.middle, c.last]
                        .into_iter()
                        .flatten()
                        .all(|ride| {
                            let route = self.trips[ride.trip as usize].route;
                            [ride.from, ride.to]
                                .into_iter()
                                .all(|s| stops.contains(&s) || stops.push(s).is_ok())
                                && (routes.contains(&route) || routes.push(route).is_ok())
                        })
                    && stops.len() + routes.len() <= 32;
                if !fits {
                    let cut = arrive(&c).saturating_sub(1);
                    window = window.min(u16::try_from(cut).unwrap_or(u16::MAX));
                    break;
                }
                used_stops = stops;
                used_routes = routes;
            }
            if window < self.horizon {
                // Equal arrivals at the cut were partly added; rebuild exactly.
                used_stops.clear();
                used_routes.clear();
                for c in self
                    .candidates
                    .iter()
                    .filter(|c| arrive(c) <= u32::from(window))
                {
                    for ride in [Some(c.first), c.middle, c.last].into_iter().flatten() {
                        for s in [ride.from, ride.to] {
                            if !used_stops.contains(&s) {
                                used_stops.push(s).map_err(|_| Error::IndexFull)?;
                            }
                        }
                        let route = self.trips[ride.trip as usize].route;
                        if !used_routes.contains(&route) {
                            used_routes.push(route).map_err(|_| Error::IndexFull)?;
                        }
                    }
                }
            }
            (window, used_stops, used_routes)
        };
        let mut names = [u8::MAX; 32];
        let mut lines = [u8::MAX; 32];
        source
            .scan(
                &[Table::Stops, Table::Routes],
                self.manifest.as_ref(),
                workspace,
                |row| {
                    if row.table() == Table::Stops {
                        let stop = row.stop()?;
                        if let Some(i) = used_stops
                            .iter()
                            .position(|&s| self.stops[s as usize].as_str() == stop.id)
                        {
                            if names[i] != u8::MAX {
                                return Err(Error::InvalidField("duplicate stop_id"));
                            }
                            let mut name = bounded_label::<64>(stop.name.unwrap_or(stop.id));
                            let members = self.endpoints[used_stops[i] as usize];
                            let place = self.places[used_stops[i] as usize];
                            let grouped = members != 0
                                && self.endpoints.iter().filter(|&&m| m & members != 0).count() > 1
                                || place != NONE
                                    && self.places.iter().filter(|&&p| p == place).count() > 1;
                            if grouped && let Some(code) = stop.code.filter(|code| !code.is_empty())
                            {
                                // Reserve room for the actual platform without changing the
                                // one-place endpoint or the fixed shared label arena.
                                let code = bounded_label::<16>(code);
                                while name.len() + " · ".len() + code.len() > 64 {
                                    name.pop();
                                }
                                let _ = name.push_str(" · ");
                                let _ = name.push_str(&code);
                            }
                            if name.is_empty() {
                                return Err(Error::InvalidField("empty journey stop name"));
                            }
                            names[i] = labels.insert(&name)?;
                        }
                    } else {
                        let route = row.route()?;
                        let k = key(route.id.as_bytes());
                        if let Some(i) = used_routes
                            .iter()
                            .position(|&r| self.routes[r as usize] == k)
                        {
                            if lines[i] != u8::MAX {
                                return Err(Error::InvalidField("duplicate route_id"));
                            }
                            let line = bounded_label::<8>(if route.short_name.is_empty() {
                                route.id
                            } else {
                                route.short_name
                            });
                            if line.is_empty() {
                                return Err(Error::InvalidField("empty journey line"));
                            }
                            lines[i] = labels.insert(&line)?;
                        }
                    }
                    Ok(())
                },
            )
            .await?;
        if names[..used_stops.len()].contains(&u8::MAX)
            || lines[..used_routes.len()].contains(&u8::MAX)
        {
            return Err(Error::InvalidField("unknown journey stop or route"));
        }
        if let Some(slot) = output.windows.get_mut(group as usize) {
            *slot = (*slot).min(window);
        }
        if let Some(slot) = output.limits.get_mut(group as usize) {
            *slot |= self.limits
                | if window < self.horizon {
                    LIMIT_WINDOW
                } else {
                    0
                };
        }
        for &c in self
            .candidates
            .iter()
            .filter(|c| arrive(c) <= u32::from(window))
        {
            let mut path = Path {
                group,
                origin: c.first.endpoint,
                target: c.target,
                walk: config.origins[c.first.endpoint as usize].walk,
                egress: config.targets[c.target as usize].walk,
                len: c.len(),
                leave: c.first.departure
                    - u16::from(config.origins[c.first.endpoint as usize].walk) * 60,
                arrival: c.end().arrival + u16::from(config.targets[c.target as usize].walk) * 60,
                ..Path::default()
            };
            for (i, ride) in [Some(c.first), c.middle, c.last]
                .into_iter()
                .flatten()
                .enumerate()
            {
                let label = |stop| used_stops.iter().position(|&s| s == stop).unwrap();
                let route = used_routes
                    .iter()
                    .position(|&r| r == self.trips[ride.trip as usize].route)
                    .unwrap();
                path.legs[i] = Leg {
                    from: names[label(ride.from)],
                    to: names[label(ride.to)],
                    line: lines[route],
                    flags: (u8::from(ride.day & REQUEST_BOARD != 0) * LEG_REQUEST_BOARD)
                        | (u8::from(ride.day & REQUEST_ALIGHT != 0) * LEG_REQUEST_ALIGHT),
                    departure: ride.departure,
                    arrival: ride.arrival,
                };
            }
            output.paths.push(path).map_err(|_| Error::IndexFull)?;
        }
        Ok(())
    }
}
fn bounded_label<const N: usize>(value: &str) -> String<N> {
    let mut text = String::new();
    for word in value.split_whitespace() {
        if !text.is_empty() && text.push(' ').is_err() {
            break;
        }
        for c in word.chars().filter(|c| !c.is_control()) {
            if text.push(c).is_err() {
                return text;
            }
        }
    }
    text
}

// Request stops are ordinary stops where the rider signals the driver; the
// leg records which end needs that so the display can say so.
fn ride_day(board: Call, alight: Call) -> u8 {
    board.day
        | if board.flags & 4 != 0 {
            REQUEST_BOARD
        } else {
            0
        }
        | if alight.flags & 8 != 0 {
            REQUEST_ALIGHT
        } else {
            0
        }
}
fn first_rides<'a, const S: usize>(
    calls: &'a [Call],
    endpoints: &'a [u8; S],
    config: &'a Journey,
) -> impl Iterator<Item = Ride> + 'a {
    calls.iter().enumerate().flat_map(move |(i, board)| {
        config
            .origins
            .iter()
            .enumerate()
            .filter(move |(origin, s)| {
                board.flags & 1 != 0
                    && endpoints[board.stop as usize] & (1 << origin) != 0
                    && board.departure >= u16::from(s.walk) * 60
            })
            .flat_map(move |(origin, _)| {
                calls[i + 1..]
                    .iter()
                    .take_while(move |alight| (alight.trip, alight.day) == (board.trip, board.day))
                    .filter(|alight| alight.flags & 2 != 0)
                    .map(move |alight| Ride {
                        trip: board.trip,
                        from: board.stop,
                        to: alight.stop,
                        departure: board.departure,
                        arrival: alight.arrival,
                        endpoint: origin as u8,
                        day: ride_day(*board, *alight),
                    })
            })
    })
}
fn last_rides<'a, const S: usize>(
    calls: &'a [Call],
    endpoints: &'a [u8; S],
    config: &'a Journey,
) -> impl Iterator<Item = Ride> + 'a {
    calls
        .iter()
        .enumerate()
        .filter(|(_, board)| board.flags & 1 != 0)
        .flat_map(move |(i, board)| {
            calls[i + 1..]
                .iter()
                .take_while(move |alight| (alight.trip, alight.day) == (board.trip, board.day))
                .filter(|alight| alight.flags & 2 != 0)
                .flat_map(move |alight| {
                    (0..config.targets.len())
                        .filter(move |&target| {
                            endpoints[alight.stop as usize] & (1 << (target + 4)) != 0
                        })
                        .map(move |target| Ride {
                            trip: board.trip,
                            from: board.stop,
                            to: alight.stop,
                            departure: board.departure,
                            arrival: alight.arrival,
                            endpoint: target as u8,
                            day: ride_day(*board, *alight),
                        })
                })
        })
}

// Identity retains accents; search may be accent-insensitive. Match the browser
// place key without allocating, truncating names or assuming feed row order.
fn place_chars(value: &str) -> impl Iterator<Item = char> + '_ {
    value.split_whitespace().enumerate().flat_map(|(i, word)| {
        (i != 0)
            .then_some(' ')
            .into_iter()
            .chain(word.chars().flat_map(char::to_lowercase))
    })
}
fn name_key(stop: gtfs::Stop<'_>) -> Result<[u32; 2], Error> {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut empty = true;
    for c in place_chars(stop.name.unwrap_or(stop.id)) {
        if c.is_control() {
            return Err(Error::InvalidField("journey stop name"));
        }
        let mut bytes = [0; 4];
        let folded = fold(hash, c.encode_utf8(&mut bytes).as_bytes());
        hash = u64::from(folded[0]) | (u64::from(folded[1]) << 32);
        empty = false;
    }
    if empty {
        return Err(Error::InvalidField("empty journey stop name"));
    }
    Ok([hash as u32, (hash >> 32) as u32])
}
fn place_key(stop: gtfs::Stop<'_>) -> Result<PlaceKey, Error> {
    Ok(if let Some(parent) = stop.parent_station {
        PlaceKey {
            tag: b'P',
            hash: key(parent.as_bytes()),
            lat: 0,
            lon: 0,
        }
    } else if let Some((lat, lon)) = coordinates(stop) {
        PlaceKey {
            tag: b'N',
            hash: name_key(stop)?,
            lat,
            lon,
        }
    } else {
        // Without a station or position, equal text cannot establish that
        // two platforms are the same physical place.
        PlaceKey {
            tag: b'I',
            hash: key(stop.id.as_bytes()),
            lat: 0,
            lon: 0,
        }
    })
}
/// The filter's place: a declared station or a normalized name. It is wider
/// than PlaceKey, so it can only add candidate stops.
fn place_filter_key(stop: gtfs::Stop<'_>) -> [u32; 2] {
    match stop.parent_station {
        Some(parent) => fold(0x5045_4e54_0000_0001, parent.as_bytes()),
        None => name_key(stop).unwrap_or(key(stop.id.as_bytes())),
    }
}
/// Row identities computed once per stop row for cohort membership.
struct Cohort {
    parent: Option<[u32; 2]>,
    id: [u32; 2],
    name: Option<[u32; 2]>,
}
impl Cohort {
    fn of(stop: gtfs::Stop<'_>) -> Self {
        Self {
            parent: stop.parent_station.map(|p| key(p.as_bytes())),
            id: key(stop.id.as_bytes()),
            name: coordinates(stop).and_then(|_| name_key(stop).ok()),
        }
    }
    fn contains(&self, key: PlaceKey) -> bool {
        match key.tag {
            b'P' => self.parent == Some(key.hash),
            b'I' => self.parent.is_none() && self.id == key.hash,
            b'N' => self.parent.is_none() && self.name == Some(key.hash),
            _ => false,
        }
    }
}

// Quantized coordinates and the same explicit distance polynomial are used by
// the browser. Partition decisions do not depend on target float precision or
// a platform-specific trigonometry approximation.
fn coordinates(stop: gtfs::Stop<'_>) -> Option<(i32, i32)> {
    let (lat, lon) = (stop.latitude?, stop.longitude?);
    (lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0)
        .then_some(((lat * 1_000_000.0) as i32, (lon * 1_000_000.0) as i32))
}
fn same_place_keys(a: PlaceKey, b: PlaceKey) -> bool {
    if a.tag != b.tag || a.hash != b.hash {
        return false;
    }
    if a.tag != b'N' {
        return true;
    }
    let radians = f64::from(a.lat + b.lat) * core::f64::consts::PI / 360_000_000.0;
    let x2 = radians * radians;
    let cos = 1.0
        + x2 * (-0.5
            + x2 * (1.0 / 24.0 + x2 * (-1.0 / 720.0 + x2 * (1.0 / 40320.0 - x2 / 3628800.0))));
    let delta = (a.lon - b.lon).abs();
    let delta = if delta > 180_000_000 {
        360_000_000 - delta
    } else {
        delta
    };
    let dx = f64::from(delta) * (111195.0 / 1_000_000.0) * cos;
    let dy = f64::from(a.lat - b.lat) * (111195.0 / 1_000_000.0);
    dx * dx + dy * dy <= 125.0 * 125.0
}
