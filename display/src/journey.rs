//! Exact, bounded timetable journeys: access walk, up to three rides, egress walk.
//! The device builds a departure/arrival profile for a six-hour window. Rendering
//! only selects among that profile; it never asks a routing service for a path.
use crate::{clock, config::Journey, feed::Tables};
use gtfs::{Date, Error, Id, Table, csv::Column as C, zip};
use heapless::{String, Vec};

pub const HORIZON: i64 = 6 * 3600;
pub const MAX_PATHS: usize = 64;
const MAX_TRIPS: usize = 1024;
const ID_BYTES: usize = 16384;
const MAX_CALLS: usize = 2048;
const MAX_STOPS: usize = 384;
const NONE: u16 = u16::MAX;
const CONDITIONAL: u8 = 0x80;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Leg {
    pub line: u8,
    pub from: u8,
    pub to: u8,
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
    pub paths: Vec<Path, MAX_PATHS>,
}
impl Cache {
    pub fn reset(&mut self, now: i64) {
        self.epoch = now;
        self.until = now + HORIZON;
        self.paths.clear();
    }
    pub fn best(&self, group: usize, now: i64) -> Option<&Path> {
        if now < self.epoch || now > self.until {
            return None;
        }
        self.paths
            .iter()
            .filter(|p| p.group as usize == group && self.epoch + i64::from(p.leave) >= now)
            .min_by_key(|p| (p.arrival, p.len, p.walk, core::cmp::Reverse(p.leave)))
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
        put(b"TTJNY002")?;
        put(&self.epoch.to_le_bytes())?;
        put(&self.until.to_le_bytes())?;
        put(&[0, 0, self.paths.len() as u8, 0])?;
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
                put(&[leg.line, leg.from, leg.to, 0])?;
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
        if take(8)? != b"TTJNY002" {
            return Err(invalid);
        }
        let epoch = i64::from_le_bytes(take(8)?.try_into().unwrap());
        let until = i64::from_le_bytes(take(8)?.try_into().unwrap());
        if !clock::valid_utc(epoch) || until.checked_sub(epoch) != Some(HORIZON) {
            return Err(invalid);
        }
        let counts = take(4)?;
        if counts[3] != 0 {
            return Err(invalid);
        }
        let mut cache = Self::default();
        cache.reset(epoch);
        if counts[0] != 0 || counts[1] != 0 {
            return Err(invalid);
        }
        for _ in 0..counts[2] {
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
                || path.arrival > HORIZON as u16
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
                    departure: u16::from_le_bytes(leg[4..6].try_into().unwrap()),
                    arrival: u16::from_le_bytes(leg[6..8].try_into().unwrap()),
                };
                if leg[3] != 0 {
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

#[derive(Clone, Copy)]
struct Trip {
    offset: u16,
    length: u8,
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
    id: Id<32>,
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

/// All transient indexes are caller-owned, allocation-free and reused per group.
pub struct Search<
    const CALLS: usize,
    const STOPS: usize,
    const TRIPS: usize,
    const IDS: usize,
    const MIDDLE: usize,
    const SERVICES: usize,
    const ROUTES: usize,
> {
    trips: Vec<Trip, TRIPS>,
    ids: Vec<u8, IDS>,
    lookup: Vec<u16, TRIPS>,
    stops: Vec<Id<32>, STOPS>,
    stop_lookup: Vec<u16, STOPS>,
    parents: [u16; STOPS],
    places: [u16; STOPS],
    routes: Vec<Id<32>, ROUTES>,
    middle_calls: Vec<Call, MIDDLE>,
    rules: Vec<Rule, 128>,
    services: Vec<Service, SERVICES>,
    candidates: Vec<Candidate, MAX_PATHS>,
    dates: [Date; 3],
    epochs: [i64; 3],
    now: i64,
    manifest: Option<zip::Manifest>,
    // One bit per origin (low nibble) and target (high nibble). Platforms
    // share an endpoint without consuming additional configured-stop slots.
    endpoints: [u8; STOPS],
}
pub type Planner = Search<MAX_CALLS, MAX_STOPS, MAX_TRIPS, ID_BYTES, 2048, 64, 64>;
pub type DevicePlanner = Search<1024, 128, 256, 4096, 128, 16, 32>;
impl<
    const CALLS: usize,
    const STOPS: usize,
    const TRIPS: usize,
    const IDS: usize,
    const MIDDLE: usize,
    const SERVICES: usize,
    const ROUTES: usize,
> Search<CALLS, STOPS, TRIPS, IDS, MIDDLE, SERVICES, ROUTES>
{
    pub fn new(today: Date) -> Self {
        Self {
            trips: Vec::new(),
            ids: Vec::new(),
            lookup: Vec::new(),
            stops: Vec::new(),
            stop_lookup: Vec::new(),
            parents: [NONE; STOPS],
            places: [NONE; STOPS],
            routes: Vec::new(),
            middle_calls: Vec::new(),
            rules: Vec::new(),
            services: Vec::new(),
            candidates: Vec::new(),
            dates: [today; 3],
            epochs: [0; 3],
            now: 0,
            manifest: None,
            endpoints: [0; STOPS],
        }
    }
    fn id(&self, trip: usize) -> &str {
        let t = self.trips[trip];
        core::str::from_utf8(&self.ids[t.offset as usize..t.offset as usize + t.length as usize])
            .unwrap()
    }
    fn trip(&mut self, id: &str, kind: u8) -> Result<u16, Error> {
        if let Some(i) = (0..self.trips.len()).find(|&i| self.id(i) == id) {
            self.trips[i].kind |= kind;
            return Ok(i as u16);
        }
        if id.is_empty() || id.len() > 255 {
            return Err(Error::IdTooLong);
        }
        let offset = self.ids.len() as u16;
        self.ids
            .extend_from_slice(id.as_bytes())
            .map_err(|_| Error::IndexFull)?;
        self.trips
            .push(Trip {
                offset,
                length: id.len() as u8,
                kind,
                active: 0,
                route: 0,
                seen: false,
            })
            .map_err(|_| Error::IndexFull)?;
        Ok((self.trips.len() - 1) as u16)
    }
    fn index_trips(&mut self) {
        self.lookup.clear();
        for i in 0..self.trips.len() {
            self.lookup.push(i as u16).unwrap();
        }
        let trips = &self.trips;
        let ids = &self.ids;
        self.lookup.sort_unstable_by(|&a, &b| {
            let bytes = |i: u16| {
                let t = trips[i as usize];
                &ids[t.offset as usize..t.offset as usize + t.length as usize]
            };
            bytes(a).cmp(bytes(b))
        });
    }
    fn find_trip(&self, id: &str) -> Option<usize> {
        self.lookup
            .binary_search_by(|&i| self.id(i as usize).cmp(id))
            .ok()
            .map(|i| self.lookup[i] as usize)
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
    fn endpoint_key(&mut self, stop: gtfs::Stop<'_>) -> Result<(usize, usize), Error> {
        let start = self.ids.len();
        if let Some(parent) = stop.parent_station {
            self.ids.push(b'P').map_err(|_| Error::IndexFull)?;
            self.ids
                .extend_from_slice(parent.as_bytes())
                .map_err(|_| Error::IndexFull)?;
        } else if let Some((lat, lon)) = coordinates(stop) {
            self.ids.push(b'N').map_err(|_| Error::IndexFull)?;
            for bytes in [lat.to_le_bytes(), lon.to_le_bytes()] {
                self.ids
                    .extend_from_slice(&bytes)
                    .map_err(|_| Error::IndexFull)?;
            }
            let before = self.ids.len();
            for c in place_chars(stop.name.unwrap_or(stop.id)) {
                if c.is_control() {
                    return Err(Error::InvalidField("journey stop name"));
                }
                let mut bytes = [0; 4];
                self.ids
                    .extend_from_slice(c.encode_utf8(&mut bytes).as_bytes())
                    .map_err(|_| Error::IndexFull)?;
            }
            if before == self.ids.len() {
                return Err(Error::InvalidField("empty journey stop name"));
            }
        } else {
            // Without a station or position, equal text cannot establish that
            // two platforms are the same physical place.
            self.ids.push(b'I').map_err(|_| Error::IndexFull)?;
            self.ids
                .extend_from_slice(stop.id.as_bytes())
                .map_err(|_| Error::IndexFull)?;
        }
        Ok((start, self.ids.len()))
    }
    fn route(&mut self, id: &str) -> Result<u8, Error> {
        if let Some(i) = self.routes.iter().position(|r| r.as_str() == id) {
            return Ok(i as u8);
        }
        self.routes
            .push(Id::new(id)?)
            .map_err(|_| Error::IndexFull)?;
        Ok((self.routes.len() - 1) as u8)
    }
    fn service(&mut self, id: &str) -> Result<&mut Service, Error> {
        let i = if let Some(i) = self.services.iter().position(|s| s.id.as_str() == id) {
            i
        } else {
            self.services
                .push(Service {
                    id: Id::new(id)?,
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
        (0..=HORIZON).contains(&at).then_some(at as u16)
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
        let mut seen = [0u8; 48];
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
        expand: bool,
    ) -> Result<(), Error> {
        let manifest = self.manifest;
        let ids_end = self.ids.len();
        self.stop_lookup.clear();
        for _ in 0..self.stops.len() {
            self.stop_lookup.push(NONE).map_err(|_| Error::IndexFull)?;
        }
        // The unused tail of the trip-ID arena holds exact names temporarily;
        // the stop lookup is also idle in this phase. Neither adds a RAM arena.
        source
            .scan(&[Table::Stops], manifest.as_ref(), workspace, |row| {
                let stop = row.stop()?;
                if stop.location_type != 0 {
                    return Ok(());
                }
                let Some(index) = self.stop_index(stop.id) else {
                    return Ok(());
                };
                if let Some(previous) = self.named_place(stop) {
                    self.places[index as usize] = self.places[previous];
                    self.stop_lookup[index as usize] = self.stop_lookup[previous];
                } else {
                    let offset = self.ids.len();
                    self.ids
                        .extend_from_slice(&[0, 0])
                        .map_err(|_| Error::IndexFull)?;
                    let (start, end) = self.endpoint_key(stop)?;
                    let length = u16::try_from(end - start).map_err(|_| Error::IndexFull)?;
                    self.ids[offset..offset + 2].copy_from_slice(&length.to_le_bytes());
                    self.stop_lookup[index as usize] =
                        u16::try_from(offset).map_err(|_| Error::IndexFull)?;
                    self.places[index as usize] = index;
                }
                Ok(())
            })
            .await?;
        if expand {
            source
                .scan(&[Table::Stops], manifest.as_ref(), workspace, |row| {
                    let stop = row.stop()?;
                    if stop.location_type == 0
                        && let Some(previous) = self.named_place(stop)
                    {
                        let place = self.places[previous];
                        let index = self.stop(stop.id)?;
                        self.places[index as usize] = place;
                    }
                    Ok(())
                })
                .await?;
        }
        self.ids.truncate(ids_end);
        self.stop_lookup.clear();
        Ok(())
    }
    fn named_place(&self, stop: gtfs::Stop<'_>) -> Option<usize> {
        self.stop_lookup.iter().position(|&offset| {
            if offset == NONE {
                return false;
            }
            let at = offset as usize;
            let length = u16::from_le_bytes(self.ids[at..at + 2].try_into().unwrap()) as usize;
            matches_place(&self.ids[at + 2..at + 2 + length], stop)
        })
    }
    fn join(&self, a: Ride, b: Ride, slack: u8) -> bool {
        (a.trip != b.trip || a.day & !CONDITIONAL != b.day & !CONDITIONAL)
            && self.transfer(a.to, b.from, slack).is_some_and(|seconds| {
                u32::from(a.arrival) + u32::from(seconds) <= u32::from(b.departure)
            })
    }
    fn candidate(
        candidates: &mut Vec<Candidate, MAX_PATHS>,
        c: Candidate,
        config: &Journey,
    ) -> Result<(), Error> {
        let walk = u16::from(config.origins[c.first.endpoint as usize].walk) * 60;
        let egress = u16::from(config.targets[c.target as usize].walk) * 60;
        let Some(leave) = c.first.departure.checked_sub(walk) else {
            return Ok(());
        };
        let arrival = u32::from(c.end().arrival) + u32::from(egress);
        if arrival > HORIZON as u32 {
            return Ok(());
        }
        if [Some(c.first), c.middle, c.last]
            .into_iter()
            .flatten()
            .any(|ride| ride.day & CONDITIONAL != 0)
        {
            return Err(Error::InvalidField(
                "conditional journey boarding or alighting is unsupported",
            ));
        }
        let dominates = |a: Candidate, b: Candidate| {
            let a_walk = u16::from(config.origins[a.first.endpoint as usize].walk) * 60;
            let b_walk = u16::from(config.origins[b.first.endpoint as usize].walk) * 60;
            let a_arrive =
                u32::from(a.end().arrival) + u32::from(config.targets[a.target as usize].walk) * 60;
            let b_arrive =
                u32::from(b.end().arrival) + u32::from(config.targets[b.target as usize].walk) * 60;
            a.target == b.target
                && a.first.departure.saturating_sub(a_walk)
                    >= b.first.departure.saturating_sub(b_walk)
                && a_arrive <= b_arrive
                && (a_arrive < b_arrive || (a.len(), a_walk) <= (b.len(), b_walk))
        };
        if candidates.iter().any(|&old| dominates(old, c)) {
            return Ok(());
        }
        candidates.retain(|&old| !dominates(c, old));
        let _ = (leave, arrival);
        candidates.push(c).map_err(|_| Error::IndexFull)
    }
    fn endpoint_paths(&mut self, calls: &mut [Call], config: &Journey) -> Result<(), Error> {
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
            if config.max_transfers != 0 {
                for last in last_rides(calls, &self.endpoints, config) {
                    if self.join(first, last, config.transfer_minutes) {
                        Self::candidate(
                            &mut self.candidates,
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
        first: usize,
    ) -> Result<(), Error> {
        self.index_trips();
        let manifest = self.manifest;
        source
            .scan(&[Table::Trips], manifest.as_ref(), workspace, |row| {
                let id = row.required(C::TripId)?;
                if let Some(i) = self.find_trip(id).filter(|&i| i >= first) {
                    if self.trips[i].seen {
                        return Err(Error::DuplicateTrip);
                    }
                    let trip = row.trip()?;
                    let active = self
                        .services
                        .iter()
                        .find(|s| s.id.as_str() == trip.service_id)
                        .map_or(0, |s| s.active);
                    let route = self.route(trip.route_id)?;
                    self.trips[i].active = active;
                    self.trips[i].route = route;
                    self.trips[i].seen = true;
                }
                Ok(())
            })
            .await?;
        if self.trips[first..].iter().any(|t| !t.seen) {
            return Err(Error::InvalidField("unknown trip_id"));
        }
        Ok(())
    }
    async fn collect_calls<const N: usize>(
        &mut self,
        source: &mut impl Tables,
        workspace: &mut zip::Workspace,
        middle: bool,
        calls: &mut Vec<Call, N>,
    ) -> Result<(), Error> {
        if middle {
            self.middle_calls.clear();
        } else {
            calls.clear();
        }
        let manifest = self.manifest;
        source
            .scan(&[Table::StopTimes], manifest.as_ref(), workspace, |row| {
                let id = row.required(C::TripId)?;
                let Some(i) = self.find_trip(id).filter(|&i| {
                    self.trips[i].active != 0 && (!middle || self.trips[i].kind & 4 != 0)
                }) else {
                    return Ok(());
                };
                let call = row.stop_time()?;
                let stop = if middle {
                    let Some(s) = self.indexed_stop(call.stop_id) else {
                        return Ok(());
                    };
                    s
                } else {
                    self.stop(call.stop_id)?
                };
                for day in 0..3 {
                    if self.trips[i].active & (1 << day) == 0 {
                        continue;
                    }
                    let (Some(arrival), Some(departure)) = (call.arrival, call.departure) else {
                        continue;
                    };
                    let arrival = self.epochs[day] + i64::from(arrival.seconds()) - self.now;
                    let departure = self.epochs[day] + i64::from(departure.seconds()) - self.now;
                    if arrival > departure {
                        return Err(Error::InvalidTime);
                    }
                    if departure < 0 || arrival > HORIZON {
                        continue;
                    }
                    let board = (0..=HORIZON).contains(&departure);
                    let alight = (0..=HORIZON).contains(&arrival);
                    // A dwell can straddle either boundary. Only the in-window
                    // timestamp grants boarding/alighting; clamping never invents it.
                    let flags = u8::from(board && call.pickup_type != 1)
                        | (u8::from(alight && call.drop_off_type != 1) * 2)
                        | (u8::from(call.pickup_type >= 2) * 4)
                        | (u8::from(call.drop_off_type >= 2) * 8);
                    let arrival = arrival.clamp(0, HORIZON) as u16;
                    let departure = departure.clamp(0, HORIZON) as u16;
                    let value = Call {
                        trip: i as u16,
                        stop,
                        sequence: call.sequence,
                        arrival,
                        departure,
                        day: day as u8,
                        flags,
                    };
                    if middle {
                        self.middle_calls
                            .push(value)
                            .map_err(|_| Error::IndexFull)?;
                    } else {
                        calls.push(value).map_err(|_| Error::IndexFull)?;
                    }
                }
                Ok(())
            })
            .await?;
        Ok(())
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
        self.ids.clear();
        self.stops.clear();
        self.parents.fill(NONE);
        self.places.fill(NONE);
        self.routes.clear();
        calls.clear();
        self.middle_calls.clear();
        self.rules.clear();
        self.services.clear();
        self.candidates.clear();
        self.manifest = None;
        self.now = now;
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
        // Trip IDs are not loaded yet: reuse their existing byte arena for the
        // full normalized endpoint names, rather than retaining another buffer.
        let mut names = [(0usize, 0usize); 8];
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
                                names[i] = self.endpoint_key(stop)?;
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
                                names[i + 4] = self.endpoint_key(stop)?;
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
        let manifest = self.manifest;
        source
            .scan(&[Table::Stops], manifest.as_ref(), workspace, |row| {
                let stop = row.stop()?;
                if stop.location_type != 0 {
                    return Ok(());
                }
                let mut members = 0;
                for (i, &(start, end)) in names.iter().enumerate() {
                    if start != end && matches_place(&self.ids[start..end], stop) {
                        members |= 1 << i;
                    }
                }
                if members != 0 {
                    let index = self.stop(stop.id)?;
                    self.endpoints[index as usize] = members;
                }
                Ok(())
            })
            .await?;
        self.ids.clear();
        self.index_stops();
        source
            .scan(&[Table::StopTimes], manifest.as_ref(), workspace, |row| {
                let stop = row.required(C::StopId)?;
                let members = self
                    .indexed_stop(stop)
                    .map_or(0, |i| self.endpoints[i as usize]);
                let kind = u8::from(members & 15 != 0) | (u8::from(members & 240 != 0) * 2);
                if kind == 0 {
                    return Ok(());
                }
                let call = row.stop_time()?;
                if (0..3).any(|day| {
                    mask & (1 << day) != 0
                        && (self.offset(call.arrival, day).is_some()
                            || self.offset(call.departure, day).is_some())
                }) {
                    self.trip(call.trip_id, kind)?;
                }
                Ok(())
            })
            .await?;
        self.metadata(source, workspace, 0).await?;
        self.collect_calls(source, workspace, false, calls).await?;
        if config.max_transfers != 0 {
            self.stations(source, workspace, false).await?;
        }
        if config.max_transfers != 0 {
            let endpoint_stops = self.stops.len();
            source
                .scan(&[Table::Transfers], manifest.as_ref(), workspace, |row| {
                    let from = row.required(C::FromStopId)?;
                    let to = row.required(C::ToStopId)?;
                    if ![from, to].iter().any(|s| {
                        self.stop_index(s)
                            .is_some_and(|i| (i as usize) < endpoint_stops)
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
        }
        if config.max_transfers != 0 {
            self.group_places(source, workspace, false).await?;
        }
        self.endpoint_paths(calls, config)?;
        if config.max_transfers == 2 {
            // Every change has one side on an already collected endpoint trip:
            // first.to -> middle.from, then middle.to -> last.from. Therefore
            // an applicable rule already touches the earlier endpoint stop set.
            self.stations(source, workspace, true).await?;
            self.group_places(source, workspace, true).await?;
            // Same-name expansion can introduce platforms in other declared
            // stations. Preserve their parent-specific transfer restrictions.
            self.stations(source, workspace, false).await?;
        }
        if config.max_transfers == 2
            && first_rides(calls, &self.endpoints, config).next().is_some()
            && last_rides(calls, &self.endpoints, config).next().is_some()
        {
            self.index_stops();
            let mut ready = [None::<u16>; STOPS];
            for (stop, value) in ready.iter_mut().enumerate().take(self.stops.len()) {
                *value = first_rides(calls, &self.endpoints, config)
                    .filter_map(|a| {
                        self.transfer(a.to, stop as u16, config.transfer_minutes)
                            .and_then(|s| a.arrival.checked_add(s))
                    })
                    .min();
            }
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
                            && self.offset(call.departure, day).is_some_and(|departure| {
                                ready[stop as usize].is_some_and(|at| at <= departure)
                            })
                    }) {
                        self.trip(call.trip_id, 4)?;
                    }
                    Ok(())
                })
                .await?;
            // Existing endpoint trips can also be the middle ride.
            for t in &mut self.trips {
                t.seen = false;
                t.active = 0;
            }
            self.metadata(source, workspace, 0).await?;
            self.collect_calls(source, workspace, true, calls).await?;
            self.middle_calls
                .sort_unstable_by_key(|c| (c.trip, c.day, c.sequence));
            for i in 0..self.middle_calls.len() {
                let board = self.middle_calls[i];
                if board.flags & 1 == 0 {
                    continue;
                }
                for j in i + 1..self.middle_calls.len() {
                    let alight = self.middle_calls[j];
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
        }
        self.publish(source, workspace, config, group, output, labels)
            .await
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
        let mut used_stops = Vec::<u16, 32>::new();
        let mut used_routes = Vec::<u8, 32>::new();
        for c in &self.candidates {
            for ride in [Some(c.first), c.middle, c.last].into_iter().flatten() {
                for stop in [ride.from, ride.to] {
                    if !used_stops.contains(&stop) {
                        used_stops.push(stop).map_err(|_| Error::IndexFull)?;
                    }
                }
                let route = self.trips[ride.trip as usize].route;
                if !used_routes.contains(&route) {
                    used_routes.push(route).map_err(|_| Error::IndexFull)?;
                }
            }
        }
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
                        if let Some(i) = used_routes
                            .iter()
                            .position(|&r| self.routes[r as usize].as_str() == route.id)
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
        for c in &self.candidates {
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

// The high bit records a conditional endpoint without enlarging a ride.
// Passing a request stop while staying on the vehicle needs no arrangement.
fn ride_day(board: Call, alight: Call) -> u8 {
    board.day
        | if board.flags & 4 != 0 || alight.flags & 8 != 0 {
            CONDITIONAL
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

fn coordinates(stop: gtfs::Stop<'_>) -> Option<(f32, f32)> {
    let (lat, lon) = (stop.latitude?, stop.longitude?);
    (lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0)
        .then_some((lat as f32, lon as f32))
}
fn matches_place(key: &[u8], stop: gtfs::Stop<'_>) -> bool {
    match key.first() {
        Some(b'P') => stop
            .parent_station
            .is_some_and(|parent| parent.as_bytes() == &key[1..]),
        Some(b'I') => stop.parent_station.is_none() && stop.id.as_bytes() == &key[1..],
        Some(b'N') if key.len() >= 9 && stop.parent_station.is_none() => {
            let Some((lat, lon)) = coordinates(stop) else {
                return false;
            };
            let a = f32::from_le_bytes(key[1..5].try_into().unwrap());
            let b = f32::from_le_bytes(key[5..9].try_into().unwrap());
            let dy = (a - lat) * 111195.0;
            let delta = (b - lon).abs();
            let delta = if delta > 180.0 { 360.0 - delta } else { delta };
            let dx = delta
                * 111195.0
                * micromath::F32Ext::cos((a + lat) * core::f32::consts::PI / 360.0);
            dx * dx + dy * dy <= 125.0 * 125.0
                && core::str::from_utf8(&key[9..])
                    .is_ok_and(|name| place_chars(stop.name.unwrap_or(stop.id)).eq(name.chars()))
        }
        _ => false,
    }
}
