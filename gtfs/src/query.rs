//! Bounded indexes and queries over a cached, seekable ZIP (flash, SD or file).
//! Indexes belong to one feed snapshot; rebuild them when the feed changes.

use crate::{Date, Error, Route, Stop, StopTime, Table, Time, Trip, csv, zip};
use embedded_io_async::{Error as _, Read, Seek, SeekFrom};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Id<const N: usize = 32> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Id<N> {
    pub const EMPTY: Self = Self {
        bytes: [0; N],
        len: 0,
    };
    pub fn new(value: &str) -> Result<Self, Error> {
        if value.len() > N {
            return Err(Error::IdTooLong);
        }
        let mut id = Self::EMPTY;
        id.bytes[..value.len()].copy_from_slice(value.as_bytes());
        id.len = value.len();
        Ok(id)
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).expect("copied UTF-8 ID")
    }
}

/// Sorted unique IDs in caller-owned storage, suitable for stops or service IDs.
pub struct IdSet<'a, const N: usize = 32> {
    storage: &'a mut [Id<N>],
    len: usize,
}

impl<'a, const N: usize> IdSet<'a, N> {
    pub fn new(storage: &'a mut [Id<N>]) -> Self {
        Self { storage, len: 0 }
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn as_slice(&self) -> &[Id<N>] {
        &self.storage[..self.len]
    }
    pub fn contains(&self, id: &str) -> bool {
        self.as_slice()
            .binary_search_by(|i| i.as_str().cmp(id))
            .is_ok()
    }
    pub fn insert(&mut self, id: &str) -> Result<(), Error> {
        match self.as_slice().binary_search_by(|i| i.as_str().cmp(id)) {
            Ok(_) => Ok(()),
            Err(pos) => {
                let value = Id::new(id)?;
                if self.len == self.storage.len() {
                    return Err(Error::IndexFull);
                }
                self.storage.copy_within(pos..self.len, pos + 1);
                self.storage[pos] = value;
                self.len += 1;
                Ok(())
            }
        }
    }
    pub fn remove(&mut self, id: &str) {
        if let Ok(pos) = self.as_slice().binary_search_by(|i| i.as_str().cmp(id)) {
            self.storage.copy_within(pos + 1..self.len, pos);
            self.len -= 1;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TripSlot<const N: usize = 32> {
    id: Id<N>,
    service: u16,
    /// Index into the caller's `route_ids` selection, not a numeric GTFS route ID.
    pub route_index: usize,
    pub direction: Option<u8>,
}

impl<const N: usize> TripSlot<N> {
    pub const EMPTY: Self = Self {
        id: Id::EMPTY,
        service: u16::MAX,
        route_index: 0,
        direction: None,
    };
    pub fn id(&self) -> &str {
        self.id.as_str()
    }
}

/// Stores selected trip IDs exactly, without hashes or a heap. Lookups use binary
/// search. Capacity and maximum ID length are chosen by the caller.
pub struct TripIndex<'a, const N: usize = 32> {
    storage: &'a mut [TripSlot<N>],
    len: usize,
    sorted: bool,
}

impl<'a, const N: usize> TripIndex<'a, N> {
    pub fn new(storage: &'a mut [TripSlot<N>]) -> Self {
        Self {
            storage,
            len: 0,
            sorted: true,
        }
    }
    pub fn clear(&mut self) {
        self.len = 0;
        self.sorted = true;
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn as_slice(&self) -> &[TripSlot<N>] {
        &self.storage[..self.len]
    }
    pub fn insert(
        &mut self,
        trip_id: &str,
        route_index: usize,
        direction: Option<u8>,
    ) -> Result<(), Error> {
        self.insert_for_service(trip_id, route_index, direction, u16::MAX)
    }

    pub(crate) fn insert_for_service(
        &mut self,
        trip_id: &str,
        route_index: usize,
        direction: Option<u8>,
        service: u16,
    ) -> Result<(), Error> {
        if self.len == self.storage.len() {
            return Err(Error::IndexFull);
        }
        self.storage[self.len] = TripSlot {
            id: Id::new(trip_id)?,
            service,
            route_index,
            direction,
        };
        self.len += 1;
        self.sorted = false;
        Ok(())
    }

    pub(crate) fn retain_services(&mut self, mut active: impl FnMut(u16) -> bool) {
        let mut len = 0;
        for i in 0..self.len {
            if active(self.storage[i].service) {
                self.storage[len] = self.storage[i];
                len += 1;
            }
        }
        self.len = len;
    }

    /// Reusable selection logic for cached feeds and HTTP range streams.
    pub fn consider<const S: usize>(
        &mut self,
        trip: Trip<'_>,
        routes: &[&str],
        direction: Option<u8>,
        services: Option<&IdSet<'_, S>>,
    ) -> Result<(), Error> {
        if direction.is_some_and(|value| value > 1) {
            return Err(Error::InvalidField("direction_id"));
        }
        if let Some(route_index) = routes.iter().position(|&r| r == trip.route_id)
            && direction.is_none_or(|d| trip.direction == Some(d))
            && services.is_none_or(|s| s.contains(trip.service_id))
        {
            self.insert(trip.id, route_index, trip.direction)?;
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), Error> {
        self.storage[..self.len].sort_unstable_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        if self.storage[..self.len]
            .windows(2)
            .any(|w| w[0].id.as_str() == w[1].id.as_str())
        {
            return Err(Error::DuplicateTrip);
        }
        self.sorted = true;
        Ok(())
    }

    pub fn find(&self, trip_id: &str) -> Result<Option<&TripSlot<N>>, Error> {
        if !self.sorted {
            return Err(Error::IndexNotSorted);
        }
        Ok(self.storage[..self.len]
            .binary_search_by(|s| s.id.as_str().cmp(trip_id))
            .ok()
            .map(|i| &self.storage[i]))
    }

    /// Cache consecutive stop_times rows, including misses. GTFS need not be
    /// grouped by trip: arbitrary ordering still falls back to exact binary search.
    pub fn cursor(&self) -> Result<TripCursor<'_, 'a, N>, Error> {
        if !self.sorted {
            return Err(Error::IndexNotSorted);
        }
        Ok(TripCursor {
            index: self,
            last: Id::EMPTY,
            found: None,
            valid: false,
        })
    }
}

pub struct TripCursor<'a, 'storage, const N: usize = 32> {
    index: &'a TripIndex<'storage, N>,
    last: Id<N>,
    found: Option<&'a TripSlot<N>>,
    valid: bool,
}

impl<'a, const N: usize> TripCursor<'a, '_, N> {
    pub fn find(&mut self, id: &str) -> Result<Option<&'a TripSlot<N>>, Error> {
        if self.valid && self.last.as_str() == id {
            return Ok(self.found);
        }
        self.found = self.index.find(id)?;
        if let Ok(last) = Id::new(id) {
            self.last = last;
            self.valid = true;
        } else {
            self.valid = false;
        }
        Ok(self.found)
    }
}

/// One stop event on a selected trip. Output stays in feed order: retain and
/// sort chosen events yourself if a chronological departure board is needed.
#[derive(Debug, Clone, Copy)]
pub struct Departure<'a> {
    pub route_index: usize,
    pub direction: Option<u8>,
    pub event: StopTime<'a>,
}

/// Bounded owned event for a selected departure board; headsigns/names belong in
/// the route/stop catalogue. String lengths are checked when copying a row.
#[derive(Debug, Clone, Copy)]
pub struct DepartureSlot<const N: usize = 32> {
    pub trip_id: Id<N>,
    pub stop_id: Id<N>,
    pub arrival: Option<Time>,
    pub departure: Option<Time>,
    pub route_index: usize,
    pub direction: Option<u8>,
    pub sequence: u32,
    pub pickup_type: u8,
    pub drop_off_type: u8,
}

impl<const N: usize> DepartureSlot<N> {
    pub const EMPTY: Self = Self {
        trip_id: Id::EMPTY,
        stop_id: Id::EMPTY,
        arrival: None,
        departure: None,
        route_index: 0,
        direction: None,
        sequence: 0,
        pickup_type: 0,
        drop_off_type: 0,
    };
}

/// A small event cache. Stream larger selections to flash instead of retaining
/// an entire city's stop_times in RAM. No event is silently dropped on overflow.
pub struct DepartureBuffer<'a, const N: usize = 32> {
    storage: &'a mut [DepartureSlot<N>],
    len: usize,
}

impl<'a, const N: usize> DepartureBuffer<'a, N> {
    pub fn new(storage: &'a mut [DepartureSlot<N>]) -> Self {
        Self { storage, len: 0 }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn as_slice(&self) -> &[DepartureSlot<N>] {
        &self.storage[..self.len]
    }
    pub fn push(&mut self, d: Departure<'_>) -> Result<(), Error> {
        if self.len == self.storage.len() {
            return Err(Error::IndexFull);
        }
        self.storage[self.len] = DepartureSlot {
            trip_id: Id::new(d.event.trip_id)?,
            stop_id: Id::new(d.event.stop_id)?,
            arrival: d.event.arrival,
            departure: d.event.departure,
            route_index: d.route_index,
            direction: d.direction,
            sequence: d.event.sequence,
            pickup_type: d.event.pickup_type,
            drop_off_type: d.event.drop_off_type,
        };
        self.len += 1;
        Ok(())
    }
    /// Missing times sort last. Repeated calls/querying the resulting slice do
    /// not need to read the source ZIP again. No allocation is used for sorting.
    pub fn sort(&mut self) {
        self.storage[..self.len].sort_unstable_by(|a, b| {
            a.departure
                .map(|t| t.seconds())
                .unwrap_or(u32::MAX)
                .cmp(&b.departure.map(|t| t.seconds()).unwrap_or(u32::MAX))
                .then_with(|| a.trip_id.as_str().cmp(b.trip_id.as_str()))
                .then(a.sequence.cmp(&b.sequence))
        });
    }
}

pub struct Feed<R> {
    source: R,
    directory: zip::Directory,
}

impl<R: Read + Seek> Feed<R> {
    pub async fn open(mut source: R, scratch: &mut [u8]) -> Result<Self, Error> {
        let directory = zip::Directory::read(&mut source, scratch).await?;
        Ok(Self { source, directory })
    }
    pub fn directory(&self) -> &zip::Directory {
        &self.directory
    }
    pub fn into_inner(self) -> R {
        self.source
    }

    pub async fn reader<'a>(
        &'a mut self,
        table: Table,
        workspace: &'a mut zip::Workspace,
    ) -> Result<zip::Reader<'a, &'a mut R>, Error> {
        if let Some(range) = self.directory.frequencies_range() {
            self.source
                .seek(SeekFrom::Start(range.start()))
                .await
                .map_err(|e| Error::Io(e.kind()))?;
            self.directory
                .validate_frequencies(&mut self.source, workspace)
                .await?;
        }
        let member = self
            .directory
            .member(table)
            .ok_or(Error::MissingTable(table))?;
        zip::Reader::member(&mut self.source, workspace, member).await
    }

    pub async fn scan(
        &mut self,
        table: Table,
        workspace: &mut zip::Workspace,
        mut visit: impl FnMut(csv::Record<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let source = self.reader(table, workspace).await?;
        let mut reader = csv::Reader::<_>::new(source, table);
        while let Some(row) = reader.next().await? {
            visit(row)?;
        }
        Ok(())
    }

    pub async fn routes(
        &mut self,
        workspace: &mut zip::Workspace,
        mut visit: impl FnMut(Route<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.scan(Table::Routes, workspace, |row| visit(row.route()?))
            .await
    }

    pub async fn stops(
        &mut self,
        workspace: &mut zip::Workspace,
        mut visit: impl FnMut(Stop<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.scan(Table::Stops, workspace, |row| visit(row.stop()?))
            .await
    }

    /// Calendar exceptions override the weekly calendar. A calendar_dates-only
    /// feed is supported. An empty result means no service on the chosen day.
    pub async fn services_on<const N: usize>(
        &mut self,
        date: Date,
        result: &mut IdSet<'_, N>,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        result.clear();
        let calendar = self.directory.member(Table::Calendar).is_some();
        let exceptions = self.directory.member(Table::CalendarDates).is_some();
        if !calendar && !exceptions {
            return Err(Error::MissingTable(Table::Calendar));
        }
        if calendar {
            self.scan(Table::Calendar, workspace, |row| {
                let c = row.calendar()?;
                if c.runs_on(date) {
                    result.insert(c.service_id)?;
                }
                Ok(())
            })
            .await?;
        }
        if exceptions {
            self.scan(Table::CalendarDates, workspace, |row| {
                let c = row.calendar_date()?;
                if c.date == date {
                    if c.added {
                        result.insert(c.service_id)?;
                    } else {
                        result.remove(c.service_id);
                    }
                }
                Ok(())
            })
            .await?;
        }
        Ok(())
    }

    /// `services=None` selects every service day (useful for all route variants).
    /// `direction=None` includes both directions and trips without direction_id.
    /// A direction other than 0 or 1 returns an error before changing `result`.
    pub async fn select_trips<const N: usize, const S: usize>(
        &mut self,
        routes: &[&str],
        direction: Option<u8>,
        services: Option<&IdSet<'_, S>>,
        result: &mut TripIndex<'_, N>,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        if direction.is_some_and(|value| value > 1) {
            return Err(Error::InvalidField("direction_id"));
        }
        result.clear();
        self.scan(Table::Trips, workspace, |row| {
            result.consider(row.trip()?, routes, direction, services)
        })
        .await?;
        result.finish()
    }

    /// Empty `stops` selects all stops. IDs are exact platform IDs, not parent
    /// stations. Pickup/drop-off restrictions and optional times are preserved.
    pub async fn departures<const N: usize>(
        &mut self,
        trips: &TripIndex<'_, N>,
        stops: &[&str],
        workspace: &mut zip::Workspace,
        mut visit: impl FnMut(Departure<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if !trips.sorted {
            return Err(Error::IndexNotSorted);
        }
        if trips.is_empty() {
            return Ok(());
        }
        let mut cursor = trips.cursor()?;
        self.scan(Table::StopTimes, workspace, |row| {
            if let Some(trip) = cursor.find(row.required(csv::Column::TripId)?)?
                && (stops.is_empty() || stops.contains(&row.required(csv::Column::StopId)?))
            {
                visit(Departure {
                    route_index: trip.route_index,
                    direction: trip.direction,
                    event: row.stop_time()?,
                })?;
            }
            Ok(())
        })
        .await
    }

    /// Union of actual stops across selected trips/variants. This is not one
    /// ordered path: use departure events' trip_id and sequence for each path.
    pub async fn route_stops<const N: usize, const S: usize>(
        &mut self,
        trips: &TripIndex<'_, N>,
        result: &mut IdSet<'_, S>,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        result.clear();
        self.departures(trips, &[], workspace, |d| result.insert(d.event.stop_id))
            .await
    }

    pub async fn named_stops<const N: usize>(
        &mut self,
        ids: &IdSet<'_, N>,
        workspace: &mut zip::Workspace,
        mut visit: impl FnMut(Stop<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.scan(Table::Stops, workspace, |row| {
            if ids.contains(row.required(csv::Column::StopId)?) {
                visit(row.stop()?)?;
            }
            Ok(())
        })
        .await
    }
}
