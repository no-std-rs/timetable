//! Forward-only official ZIP queries. No seeking, complete download buffering,
//! filesystem or mirror access is required. Stage callback output until success.
//!
//! Keep the manifest and selected trip index from mirror bootstrap for a single
//! official pass. Without metadata, `prepare` and `departures` take two forward
//! passes over the same pinned ZIP. Calendar/member ordering is arbitrary.

use crate::{
    Calendar, CalendarDate, Date, Error, Id, Table, TripIndex, csv,
    query::Departure,
    zip::{self, Manifest},
};
use embedded_io_async::Read;

/// Stream selected tables and capture identities of all recognized GTFS tables.
/// Unselected known-size entries are skipped without decompression. Their local
/// identities are checked against the central directory, not recomputed CRCs.
pub async fn scan<R: Read>(
    source: R,
    tables: &[Table],
    expected: Option<&Manifest>,
    workspace: &mut zip::Workspace,
    visit: impl FnMut(csv::Record<'_>) -> Result<(), Error>,
) -> Result<Manifest, Error> {
    scan_with_limits::<R, 8192, 48>(source, tables, expected, workspace, visit).await
}

pub async fn scan_with_limits<R: Read, const ROW: usize, const FIELDS: usize>(
    source: R,
    tables: &[Table],
    expected: Option<&Manifest>,
    workspace: &mut zip::Workspace,
    mut visit: impl FnMut(csv::Record<'_>) -> Result<(), Error>,
) -> Result<Manifest, Error> {
    let mut archive = zip::Reader::new(source, workspace);
    let mut observed = Manifest::EMPTY;
    while let Some(entry) = archive.next_entry().await? {
        if let Some(table) = entry.table() {
            if observed.get(table).is_some() {
                return Err(Error::DuplicateTable(table));
            }
            if let Some(expected) = expected {
                if expected.get(table).is_none() {
                    return Err(Error::SnapshotMismatch(table));
                }
                if let Some(actual) = entry.fingerprint()
                    && expected.get(table) != Some(actual)
                {
                    return Err(Error::SnapshotMismatch(table));
                }
            }
            if tables.contains(&table) {
                let mut reader = csv::Reader::<_, ROW, FIELDS>::new(&mut archive, table);
                while let Some(row) = reader.next().await? {
                    visit(row)?;
                }
            }
            archive.skip_entry().await?;
            let actual = archive
                .current_entry()
                .and_then(|e| e.fingerprint())
                .ok_or(Error::InvalidZip)?;
            if let Some(expected) = expected
                && expected.get(table) != Some(actual)
            {
                return Err(Error::SnapshotMismatch(table));
            }
            observed.record(table, actual)?;
        }
    }
    archive.finish_archive(&observed).await?;
    if let Some(expected) = expected {
        expected.check(&observed)?;
    }
    Ok(observed)
}

#[derive(Clone, Copy, Debug)]
pub struct Selection<'a> {
    pub routes: &'a [&'a str],
    pub direction: Option<u8>,
    /// `None` keeps all service-day variants.
    pub date: Option<Date>,
}

/// Stable service ordinals let metadata be read in any ZIP member order without
/// storing another string per trip. Exceptions always override the base calendar.
#[derive(Clone, Copy, Debug)]
pub struct ServiceSlot<const N: usize = 32> {
    id: Id<N>,
    base: bool,
    exception: Option<bool>,
    calendar_seen: bool,
}

impl<const N: usize> ServiceSlot<N> {
    pub const EMPTY: Self = Self {
        id: Id::EMPTY,
        base: false,
        exception: None,
        calendar_seen: false,
    };
}

pub struct Services<'a, const N: usize = 32> {
    storage: &'a mut [ServiceSlot<N>],
    len: usize,
}

impl<'a, const N: usize> Services<'a, N> {
    pub fn new(storage: &'a mut [ServiceSlot<N>]) -> Self {
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
    fn register(&mut self, id: &str) -> Result<u16, Error> {
        if let Some(i) = self.storage[..self.len]
            .iter()
            .position(|s| s.id.as_str() == id)
        {
            return Ok(i as u16);
        }
        if self.len == self.storage.len() || self.len >= u16::MAX as usize {
            return Err(Error::IndexFull);
        }
        let i = self.len;
        self.storage[i] = ServiceSlot {
            id: Id::new(id)?,
            ..ServiceSlot::EMPTY
        };
        self.len += 1;
        Ok(i as u16)
    }
    fn calendar(&mut self, calendar: Calendar<'_>, date: Date) -> Result<(), Error> {
        let i = self.register(calendar.service_id)? as usize;
        let slot = &mut self.storage[i];
        if slot.calendar_seen {
            return Err(Error::InvalidField("duplicate calendar service_id"));
        }
        slot.calendar_seen = true;
        slot.base = calendar.runs_on(date);
        Ok(())
    }
    fn exception(&mut self, exception: CalendarDate<'_>, date: Date) -> Result<(), Error> {
        if exception.date != date {
            return Ok(());
        }
        let i = self.register(exception.service_id)? as usize;
        if self.storage[i].exception.is_some() {
            return Err(Error::InvalidField("duplicate calendar exception"));
        }
        self.storage[i].exception = Some(exception.added);
        Ok(())
    }
    fn active(&self, i: u16) -> bool {
        self.storage
            .get(i as usize)
            .filter(|_| (i as usize) < self.len)
            .is_some_and(|s| s.exception.unwrap_or(s.base))
    }
}

/// First forward pass when no matching mirror metadata is retained. The trip
/// capacity must fit selected routes across ALL service days until the pass ends;
/// date filtering runs after calendars/exceptions have been seen in any order.
pub async fn prepare<R: Read, const N: usize, const S: usize>(
    source: R,
    selection: Selection<'_>,
    trips: &mut TripIndex<'_, N>,
    services: &mut Services<'_, S>,
    workspace: &mut zip::Workspace,
) -> Result<Manifest, Error> {
    trips.clear();
    services.clear();
    if selection.direction.is_some_and(|d| d > 1) {
        return Err(Error::InvalidField("direction_id"));
    }
    let tables = if selection.date.is_some() {
        &[Table::Trips, Table::Calendar, Table::CalendarDates][..]
    } else {
        &[Table::Trips][..]
    };
    let manifest = scan(source, tables, None, workspace, |row| {
        match row.table() {
            Table::Trips => {
                let trip = row.trip()?;
                if let Some(route) = selection.routes.iter().position(|&r| r == trip.route_id)
                    && selection
                        .direction
                        .is_none_or(|d| trip.direction == Some(d))
                {
                    let service = if selection.date.is_some() {
                        services.register(trip.service_id)?
                    } else {
                        u16::MAX
                    };
                    trips.insert_for_service(trip.id, route, trip.direction, service)?;
                }
            }
            Table::Calendar => services.calendar(row.calendar()?, selection.date.unwrap())?,
            Table::CalendarDates => {
                services.exception(row.calendar_date()?, selection.date.unwrap())?
            }
            _ => {}
        }
        Ok(())
    })
    .await?;
    for table in [Table::Trips, Table::StopTimes] {
        if manifest.get(table).is_none() {
            return Err(Error::MissingTable(table));
        }
    }
    if selection.date.is_some() {
        if manifest.get(Table::Calendar).is_none() && manifest.get(Table::CalendarDates).is_none() {
            return Err(Error::MissingTable(Table::Calendar));
        }
        trips.retain_services(|i| services.active(i));
    }
    trips.finish()?;
    Ok(manifest)
}

/// A single official ZIP pass using trip metadata from this exact snapshot.
/// It validates every known table identity and the ZIP footer before returning.
/// The callback is provisional until success, including for late mismatches.
pub async fn departures<R: Read, const N: usize>(
    source: R,
    manifest: &Manifest,
    trips: &TripIndex<'_, N>,
    stops: &[&str],
    workspace: &mut zip::Workspace,
    mut visit: impl FnMut(Departure<'_>) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut cursor = trips.cursor()?;
    if manifest.get(Table::StopTimes).is_none() {
        return Err(Error::MissingTable(Table::StopTimes));
    }
    scan(
        source,
        &[Table::StopTimes],
        Some(manifest),
        workspace,
        |row| {
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
        },
    )
    .await?;
    Ok(())
}

/// Stage a small selection in caller storage. Errors and cancellation clear the
/// buffer, including late snapshot/CRC/footer failures and capacity exhaustion.
pub async fn collect<R: Read, const N: usize, const O: usize>(
    source: R,
    manifest: &Manifest,
    trips: &TripIndex<'_, N>,
    stops: &[&str],
    workspace: &mut zip::Workspace,
    output: &mut crate::query::DepartureBuffer<'_, O>,
) -> Result<(), Error> {
    output.clear();
    let mut staged = Staged {
        output,
        committed: false,
    };
    let result = departures(source, manifest, trips, stops, workspace, |d| {
        staged.output.push(d)
    })
    .await;
    staged.committed = result.is_ok();
    result
}

struct Staged<'a, 'storage, const N: usize> {
    output: &'a mut crate::query::DepartureBuffer<'storage, N>,
    committed: bool,
}

impl<const N: usize> Drop for Staged<'_, '_, N> {
    fn drop(&mut self) {
        if !self.committed {
            self.output.clear();
        }
    }
}
