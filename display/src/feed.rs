//! Three forward passes bound RAM to active trips, irrespective of ZIP order:
//! calendars, selected active trips, then selected future stop times. Each pass
//! must match the same manifest. No new events are published on a late error.
use crate::{
    Config, Departure, clock,
    model::{Headings, MAX_DEPARTURES},
};
use embedded_io_async::Read;
use gtfs::{Date, Error, Id, Table, csv::Column, stream, zip};
use heapless::Vec;

pub const MAX_TRIPS: usize = 1024;
pub const MAX_SERVICES: usize = 32;
pub const TRIP_ID_BYTES: usize = 16 * 1024;

struct Staged<'a> {
    events: &'a mut Vec<Departure, MAX_DEPARTURES>,
    before: usize,
    committed: bool,
}
impl Drop for Staged<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.events.truncate(self.before);
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
        match self.flags >> 2 {
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
}

pub struct Builder {
    services: [Vec<Id<32>, MAX_SERVICES>; 2],
    removed: [Vec<Id<32>, MAX_SERVICES>; 2],
    trips: Vec<Trip, MAX_TRIPS>,
    trip_ids: Vec<u8, TRIP_ID_BYTES>,
    resolved: [Resolved; crate::config::MAX_GROUPS * crate::config::MAX_PICKS],
    pub events: Vec<Departure, MAX_DEPARTURES>,
    pub headsigns: Headings,
    pub dates: [Date; 2],
    pub now: i64,
    pub manifest: Option<zip::Manifest>,
}

fn insert<const N: usize>(set: &mut Vec<Id<32>, N>, id: &str) -> Result<(), Error> {
    if !set.iter().any(|v| v.as_str() == id) {
        set.push(Id::new(id)?).map_err(|_| Error::IndexFull)?;
    }
    Ok(())
}

impl Builder {
    pub fn new(today: Date) -> Self {
        Self {
            services: [Vec::new(), Vec::new()],
            removed: [Vec::new(), Vec::new()],
            trips: Vec::new(),
            trip_ids: Vec::new(),
            resolved: [Resolved {
                route: Id::EMPTY,
                stop: Id::EMPTY,
            }; crate::config::MAX_GROUPS * crate::config::MAX_PICKS],
            events: Vec::new(),
            headsigns: Headings::default(),
            dates: [clock::previous(today).unwrap_or(today), today],
            now: 0,
            manifest: None,
        }
    }
    pub fn begin(&mut self, now: i64) -> Result<(), Error> {
        let date = clock::service_date(now).ok_or(Error::InvalidDate)?;
        self.dates = [clock::previous(date).ok_or(Error::InvalidDate)?, date];
        self.now = now;
        self.events.clear();
        self.headsigns.clear();
        self.manifest = None;
        Ok(())
    }
    pub async fn calendars<R: Read>(
        &mut self,
        source: R,
        mask: u8,
        config: &Config,
        workspace: &mut zip::Workspace,
    ) -> Result<(), Error> {
        for s in &mut self.services {
            s.clear();
        }
        for s in &mut self.removed {
            s.clear();
        }
        self.trips.clear();
        self.trip_ids.clear();
        self.manifest = None;
        self.resolved.fill(Resolved {
            route: Id::EMPTY,
            stop: Id::EMPTY,
        });
        let manifest = stream::scan(
            source,
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
                                    if !resolved.route.as_str().is_empty() {
                                        return Err(Error::InvalidField("ambiguous line"));
                                    }
                                    resolved.route = Id::new(r.id)?;
                                }
                            } else {
                                let s = row.stop()?;
                                if (!pick.stop_code.is_empty()
                                    && s.code == Some(pick.stop_code.as_str()))
                                    || (pick.stop_code.is_empty()
                                        && s.id == pick.stop
                                        && s.name == Some(pick.stop_name.as_str()))
                                {
                                    if !resolved.stop.as_str().is_empty() {
                                        return Err(Error::InvalidField("ambiguous stop code"));
                                    }
                                    resolved.stop = Id::new(s.id)?;
                                }
                            }
                        }
                    }
                    return Ok(());
                }
                for day in 0..2 {
                    if mask & (1 << day) == 0 {
                        continue;
                    }
                    if row.table() == Table::Calendar {
                        let c = row.calendar()?;
                        if c.runs_on(self.dates[day])
                            && !self.removed[day].iter().any(|s| s.as_str() == c.service_id)
                        {
                            insert(&mut self.services[day], c.service_id)?;
                        }
                    } else {
                        let c = row.calendar_date()?;
                        if c.date == self.dates[day] {
                            if c.added {
                                insert(&mut self.services[day], c.service_id)?;
                            } else {
                                insert(&mut self.removed[day], c.service_id)?;
                                self.services[day].retain(|s| s.as_str() != c.service_id);
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
        let expected = self
            .manifest
            .as_ref()
            .ok_or(Error::MissingTable(Table::Trips))?;
        self.trips.clear();
        self.trip_ids.clear();
        stream::scan(source, &[Table::Trips], Some(expected), workspace, |row| {
            let t = row.trip()?;
            let mut days = 0;
            for i in 0..2 {
                if self.services[i].iter().any(|s| s.as_str() == t.service_id) {
                    days |= 1 << i;
                }
            }
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
                let headsign = self.headsigns.insert(t.headsign.unwrap_or(""))?;
                self.trips
                    .push(Trip {
                        offset,
                        length: t.id.len() as u8,
                        route,
                        flags: days | (t.direction.unwrap_or(2) << 2),
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
        Ok(())
    }
    pub async fn departures<R: Read>(
        &mut self,
        source: R,
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
        ];
        let before = self.events.len();
        let mut staged = Staged {
            events: &mut self.events,
            before,
            committed: false,
        };
        let result = stream::scan(
            source,
            &[Table::StopTimes],
            Some(expected),
            workspace,
            |row| {
                let id = row.required(Column::TripId)?;
                let Ok(index) = self
                    .trips
                    .binary_search_by(|t| t.id(&self.trip_ids).cmp(id.as_bytes()))
                else {
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
                                && at <= self.now + 6 * 3600
                            {
                                let headsign = if let Some(name) = event.headsign {
                                    self.headsigns.insert(name)?
                                } else {
                                    t.headsign
                                };
                                staged
                                    .events
                                    .push(Departure {
                                        at,
                                        group: group as u8,
                                        pick: pick as u8,
                                        conditional: event.pickup_type != 0,
                                        headsign,
                                    })
                                    .map_err(|_| Error::IndexFull)?;
                            }
                        }
                    }
                }
                Ok(())
            },
        )
        .await;
        staged.committed = result.is_ok();
        result.map(|_| ())
    }
}
