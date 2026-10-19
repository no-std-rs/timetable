//! Three forward passes bound RAM to active trips, irrespective of ZIP order:
//! calendars, selected active trips, then selected future stop times. Each pass
//! must match the same manifest. No new events are published on a late error.
use crate::{
    Config, Departure, clock,
    departures::{Departures, HORIZON_SECONDS},
    model::Headings,
};
use embedded_io_async::Read;
use gtfs::{Date, Error, Id, Table, csv::Column, stream, zip};
use heapless::Vec;

/// A validated snapshot scanner: full forward ZIP or selective immutable ranges.
#[allow(async_fn_in_trait)]
pub trait Tables {
    async fn scan(
        &mut self,
        tables: &[Table],
        expected: Option<&zip::Manifest>,
        workspace: &mut zip::Workspace,
        visit: impl FnMut(gtfs::csv::Record<'_>) -> Result<(), Error>,
    ) -> Result<zip::Manifest, Error>;
}
struct Forward<R>(Option<R>);
impl<R: Read> Tables for Forward<R> {
    async fn scan(
        &mut self,
        tables: &[Table],
        expected: Option<&zip::Manifest>,
        workspace: &mut zip::Workspace,
        visit: impl FnMut(gtfs::csv::Record<'_>) -> Result<(), Error>,
    ) -> Result<zip::Manifest, Error> {
        stream::scan(
            self.0.take().ok_or(Error::InvalidZip)?,
            tables,
            expected,
            workspace,
            visit,
        )
        .await
    }
}

pub const MAX_TRIPS: usize = 1024;
/// Distinct service IDs across yesterday, today and tomorrow, including removals.
pub const MAX_SERVICES: usize = 64;
pub const TRIP_ID_BYTES: usize = 16 * 1024;

struct Staged<'a> {
    events: &'a mut Departures,
    before: usize,
    headsigns: &'a mut Headings,
    headings_before: (usize, usize),
    committed: bool,
}
impl Drop for Staged<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.events.truncate(self.before);
            self.headsigns.restore(self.headings_before);
        }
    }
}

#[derive(Clone, Copy)]
struct Trip {
    offset: u16,
    length: u8,
    route: u8,
    flags: u8,
    headsign: u8,
}
impl Trip {
    fn id<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        &bytes[self.offset as usize..self.offset as usize + self.length as usize]
    }
    fn direction(&self) -> i8 {
        match self.flags >> 3 {
            0 => 0,
            1 => 1,
            _ => -1,
        }
    }
}
#[derive(Clone, Copy)]
struct Resolved {
    route: Id<32>,
    stop: Id<32>,
    route_exact: bool,
    route_ambiguous: bool,
    stop_exact: bool,
    stop_ambiguous: bool,
}

struct Service {
    id: Id<32>,
    active: u8,
    removed: u8,
}

pub struct Builder {
    services: Vec<Service, MAX_SERVICES>,
    trips: Vec<Trip, MAX_TRIPS>,
    trip_ids: Vec<u8, TRIP_ID_BYTES>,
    resolved: [Resolved; crate::config::MAX_GROUPS * crate::config::MAX_PICKS],
    pub events: Departures,
    pub headsigns: Headings,
    /// Mask bits 0/1/2 select yesterday/today/tomorrow for one archive.
    pub dates: [Date; 3],
    pub now: i64,
    pub manifest: Option<zip::Manifest>,
}

fn service<'a>(
    set: &'a mut Vec<Service, MAX_SERVICES>,
    id: &str,
) -> Result<&'a mut Service, Error> {
    let index = if let Some(i) = set.iter().position(|s| s.id.as_str() == id) {
        i
    } else {
        set.push(Service {
            id: Id::new(id)?,
            active: 0,
            removed: 0,
        })
        .map_err(|_| Error::IndexFull)?;
        set.len() - 1
    };
    Ok(&mut set[index])
}

// Match the catalogue's lossy human label only when no stable stop code exists.
fn stop_label(name: &str) -> heapless::String<64> {
    let mut label = heapless::String::<64>::new();
    for word in name.split_whitespace() {
        for c in (!label.is_empty())
            .then_some(' ')
            .into_iter()
            .chain(word.chars().filter(|c| *c != '\0'))
        {
            if label.push(c).is_err() {
                while label.len() > 61 {
                    label.pop();
                }
                label.push_str("...").unwrap();
                return label;
            }
        }
    }
    label
}

impl Builder {
    pub fn new(today: Date) -> Self {
        Self {
            services: Vec::new(),
            trips: Vec::new(),
            trip_ids: Vec::new(),
            resolved: [Resolved {
                route: Id::EMPTY,
                stop: Id::EMPTY,
                route_exact: false,
                route_ambiguous: false,
                stop_exact: false,
                stop_ambiguous: false,
            }; crate::config::MAX_GROUPS * crate::config::MAX_PICKS],
            events: Departures::new(0),
            headsigns: Headings::default(),
            dates: [
                clock::previous(today).unwrap_or(today),
                today,
                clock::next(today).unwrap_or(today),
            ],
            now: 0,
            manifest: None,
        }
    }
    pub fn begin(&mut self, now: i64) -> Result<(), Error> {
        let date = clock::service_date(now).ok_or(Error::InvalidDate)?;
        self.dates = [
            clock::previous(date).ok_or(Error::InvalidDate)?,
            date,
            clock::next(date).ok_or(Error::InvalidDate)?,
        ];
        self.now = now;
        self.events.reset(now);
        self.headsigns.clear();
        self.manifest = None;
        Ok(())
    }
    /// Include tomorrow when its GTFS service epoch enters the six-hour horizon.
    pub fn day_mask(&self) -> u8 {
        if clock::service_epoch(self.dates[2])
            .is_some_and(|epoch| epoch <= self.now + HORIZON_SECONDS)
        {
            7
        } else {
            3
        }
    }
    pub async fn calendars<R: Read>(
        &mut self,
        source: R,
        mask: u8,
        config: &Config,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        self.calendars_from(&mut Forward(Some(source)), mask, config, workspace)
            .await
    }
    pub async fn calendars_from<S: Tables>(
        &mut self,
        source: &mut S,
        mask: u8,
        config: &Config,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        let mask = mask & self.day_mask();
        self.services.clear();
        self.trips.clear();
        self.trip_ids.clear();
        self.manifest = None;
        self.resolved.fill(Resolved {
            route: Id::EMPTY,
            stop: Id::EMPTY,
            route_exact: false,
            route_ambiguous: false,
            stop_exact: false,
            stop_ambiguous: false,
        });
        let manifest = source
            .scan(
                &[
                    Table::Routes,
                    Table::Stops,
                    Table::Calendar,
                    Table::CalendarDates,
                ],
                None,
                workspace,
                |row| {
                    if matches!(row.table(), Table::Routes | Table::Stops) {
                        for (g, group) in config.groups.iter().enumerate() {
                            for (p, pick) in group.picks.iter().enumerate() {
                                let resolved = &mut self.resolved[g * crate::config::MAX_PICKS + p];
                                if row.table() == Table::Routes {
                                    let r = row.route()?;
                                    if r.short_name == pick.line
                                        || r.short_name.is_empty() && r.id == pick.route
                                    {
                                        if r.id == pick.route {
                                            resolved.route = Id::new(r.id)?;
                                            resolved.route_exact = true;
                                            resolved.route_ambiguous = false;
                                        } else if !resolved.route_exact {
                                            resolved.route_ambiguous |=
                                                !resolved.route.as_str().is_empty();
                                            resolved.route = Id::new(r.id)?;
                                        }
                                    }
                                } else {
                                    let s = row.stop()?;
                                    if (!pick.stop_code.is_empty()
                                        && s.code == Some(pick.stop_code.as_str()))
                                        || (pick.stop_code.is_empty()
                                            && s.id == pick.stop
                                            && s.name.is_some_and(|name| {
                                                stop_label(name) == pick.stop_name
                                            }))
                                    {
                                        if s.id == pick.stop {
                                            resolved.stop = Id::new(s.id)?;
                                            resolved.stop_exact = true;
                                            resolved.stop_ambiguous = false;
                                        } else if !resolved.stop_exact {
                                            resolved.stop_ambiguous |=
                                                !resolved.stop.as_str().is_empty();
                                            resolved.stop = Id::new(s.id)?;
                                        }
                                    }
                                }
                            }
                        }
                        return Ok(());
                    }
                    for day in 0..3 {
                        if mask & (1 << day) == 0 {
                            continue;
                        }
                        if row.table() == Table::Calendar {
                            let c = row.calendar()?;
                            if c.runs_on(self.dates[day]) {
                                let s = service(&mut self.services, c.service_id)?;
                                if s.removed & (1 << day) == 0 {
                                    s.active |= 1 << day;
                                }
                            }
                        } else {
                            let c = row.calendar_date()?;
                            if c.date == self.dates[day] {
                                let s = service(&mut self.services, c.service_id)?;
                                if c.added {
                                    s.active |= 1 << day;
                                } else {
                                    s.removed |= 1 << day;
                                    s.active &= !(1 << day);
                                }
                            }
                        }
                    }
                    Ok(())
                },
            )
            .await?;
        for table in [Table::Trips, Table::StopTimes] {
            if manifest.get(table).is_none() {
                return Err(Error::MissingTable(table));
            }
        }
        if manifest.get(Table::Calendar).is_none() && manifest.get(Table::CalendarDates).is_none() {
            return Err(Error::MissingTable(Table::Calendar));
        }
        for (g, group) in config.groups.iter().enumerate() {
            for p in 0..group.picks.len() {
                let r = self.resolved[g * crate::config::MAX_PICKS + p];
                if r.route_ambiguous {
                    return Err(Error::InvalidField("ambiguous line"));
                }
                if r.stop_ambiguous {
                    return Err(Error::InvalidField("ambiguous stop code"));
                }
                if r.route.as_str().is_empty() || r.stop.as_str().is_empty() {
                    return Err(Error::InvalidField("selection no longer exists"));
                }
            }
        }
        self.manifest = Some(manifest);
        Ok(())
    }
    pub async fn trips<R: Read>(
        &mut self,
        source: R,
        config: &Config,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        self.trips_from(&mut Forward(Some(source)), config, workspace)
            .await
    }
    pub async fn trips_from<S: Tables>(
        &mut self,
        source: &mut S,
        config: &Config,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        let expected = self
            .manifest
            .as_ref()
            .ok_or(Error::MissingTable(Table::Trips))?;
        self.trips.clear();
        self.trip_ids.clear();
        let mut staged = Staged {
            before: self.events.len(),
            events: &mut self.events,
            headings_before: self.headsigns.checkpoint(),
            headsigns: &mut self.headsigns,
            committed: false,
        };
        source
            .scan(&[Table::Trips], Some(expected), workspace, |row| {
                let t = row.trip()?;
                let days = self
                    .services
                    .iter()
                    .find(|s| s.id.as_str() == t.service_id)
                    .map_or(0, |s| s.active);
                if days == 0 {
                    return Ok(());
                }
                let mut route = None;
                for (g, group) in config.groups.iter().enumerate() {
                    for (p, pick) in group.picks.iter().enumerate() {
                        if self.resolved[g * crate::config::MAX_PICKS + p]
                            .route
                            .as_str()
                            == t.route_id
                            && (pick.direction < 0 || t.direction == Some(pick.direction as u8))
                        {
                            route = Some((g * crate::config::MAX_PICKS + p) as u8);
                            break;
                        }
                    }
                    if route.is_some() {
                        break;
                    }
                }
                if let Some(route) = route {
                    if t.id.len() > 32 {
                        return Err(Error::IdTooLong);
                    }
                    let offset = self.trip_ids.len() as u16;
                    self.trip_ids
                        .extend_from_slice(t.id.as_bytes())
                        .map_err(|_| Error::IndexFull)?;
                    let headsign = staged.headsigns.insert(t.headsign.unwrap_or(""))?;
                    self.trips
                        .push(Trip {
                            offset,
                            length: t.id.len() as u8,
                            route,
                            flags: days | (t.direction.unwrap_or(2) << 3),
                            headsign,
                        })
                        .map_err(|_| Error::IndexFull)?;
                }
                Ok(())
            })
            .await?;
        self.trips
            .sort_unstable_by(|a, b| a.id(&self.trip_ids).cmp(b.id(&self.trip_ids)));
        if self
            .trips
            .windows(2)
            .any(|w| w[0].id(&self.trip_ids) == w[1].id(&self.trip_ids))
        {
            return Err(Error::DuplicateTrip);
        }
        staged.committed = true;
        Ok(())
    }
    pub async fn departures<R: Read>(
        &mut self,
        source: R,
        config: &Config,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        self.departures_from(&mut Forward(Some(source)), config, workspace)
            .await
    }
    pub async fn departures_from<S: Tables>(
        &mut self,
        source: &mut S,
        config: &Config,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        let expected = self
            .manifest
            .as_ref()
            .ok_or(Error::MissingTable(Table::StopTimes))?;
        let epochs = [
            clock::service_epoch(self.dates[0]).ok_or(Error::InvalidDate)?,
            clock::service_epoch(self.dates[1]).ok_or(Error::InvalidDate)?,
            clock::service_epoch(self.dates[2]).ok_or(Error::InvalidDate)?,
        ];
        let before = self.events.len();
        let headings_before = self.headsigns.checkpoint();
        let mut staged = Staged {
            events: &mut self.events,
            before,
            headsigns: &mut self.headsigns,
            headings_before,
            committed: false,
        };
        // Official stop_times rows are often grouped by trip. Cache exact hits
        // and misses; an interleaved ID still falls back to binary search.
        let mut last_trip: Option<(Id<32>, Option<usize>)> = None;
        let result = source
            .scan(&[Table::StopTimes], Some(expected), workspace, |row| {
                let id = row.required(Column::TripId)?;
                let found = if let Some((key, found)) = &last_trip
                    && key.as_str() == id
                {
                    *found
                } else {
                    let found = self
                        .trips
                        .binary_search_by(|t| t.id(&self.trip_ids).cmp(id.as_bytes()))
                        .ok();
                    // Long unselected IDs remain valid and are never shortened.
                    last_trip = Id::new(id).ok().map(|key| (key, found));
                    found
                };
                let Some(index) = found else {
                    return Ok(());
                };
                let t = self.trips[index];
                let event = row.stop_time()?;
                if event.pickup_type == 1 {
                    return Ok(());
                }
                let Some(departure) = event.departure else {
                    return Ok(());
                };
                let route = self.resolved[t.route as usize].route;
                for (group, g) in config.groups.iter().enumerate() {
                    for (pick, p) in g.picks.iter().enumerate() {
                        let resolved = self.resolved[group * crate::config::MAX_PICKS + pick];
                        if resolved.route != route
                            || resolved.stop.as_str() != event.stop_id
                            || (p.direction >= 0 && t.direction() != p.direction)
                        {
                            continue;
                        }
                        for (day, epoch) in epochs.iter().enumerate() {
                            let at = epoch + i64::from(departure.seconds());
                            if t.flags & (1 << day) != 0
                                && at > self.now
                                && at <= self.now + HORIZON_SECONDS
                            {
                                let headsign = if let Some(name) = event.headsign {
                                    staged.headsigns.insert(name)?
                                } else {
                                    t.headsign
                                };
                                staged.events.push(Departure {
                                    at,
                                    group: group as u8,
                                    pick: pick as u8,
                                    conditional: event.pickup_type != 0,
                                    headsign,
                                })?;
                            }
                        }
                    }
                }
                Ok(())
            })
            .await;
        staged.committed = result.is_ok();
        result.map(|_| ())
    }
}
