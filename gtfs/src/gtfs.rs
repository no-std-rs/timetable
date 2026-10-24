//! Borrowed GTFS records, Gregorian service dates and times beyond midnight.

use crate::{
    Error,
    csv::{Column as C, Record},
};
use core::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Table {
    Routes,
    Stops,
    Trips,
    StopTimes,
    Calendar,
    CalendarDates,
    FeedInfo,
    Transfers,
}

impl Table {
    pub const COUNT: usize = 8;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Routes,
        Self::Stops,
        Self::Trips,
        Self::StopTimes,
        Self::Calendar,
        Self::CalendarDates,
        Self::FeedInfo,
        Self::Transfers,
    ];

    pub fn filename(self) -> &'static str {
        match self {
            Self::Routes => "routes.txt",
            Self::Stops => "stops.txt",
            Self::Trips => "trips.txt",
            Self::StopTimes => "stop_times.txt",
            Self::Calendar => "calendar.txt",
            Self::CalendarDates => "calendar_dates.txt",
            Self::FeedInfo => "feed_info.txt",
            Self::Transfers => "transfers.txt",
        }
    }

    pub fn from_filename(name: &[u8]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.filename().as_bytes() == name)
    }

    pub(crate) fn required(self) -> &'static [C] {
        match self {
            Self::Routes => &[C::RouteId, C::RouteType],
            Self::Stops => &[C::StopId],
            Self::Trips => &[C::RouteId, C::ServiceId, C::TripId],
            Self::StopTimes => &[C::TripId, C::StopId, C::StopSequence],
            Self::Calendar => &[
                C::ServiceId,
                C::Monday,
                C::Tuesday,
                C::Wednesday,
                C::Thursday,
                C::Friday,
                C::Saturday,
                C::Sunday,
                C::StartDate,
                C::EndDate,
            ],
            Self::CalendarDates => &[C::ServiceId, C::Date, C::ExceptionType],
            Self::FeedInfo => &[],
            Self::Transfers => &[C::FromStopId, C::ToStopId],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date(u32);

impl Date {
    pub const fn new(year: u16, month: u8, day: u8) -> Result<Self, Error> {
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days = match month {
            2 => {
                if leap {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => 0,
        };
        if year == 0 || year > 9999 || day == 0 || day > days {
            return Err(Error::InvalidDate);
        }
        Ok(Self(year as u32 * 10000 + month as u32 * 100 + day as u32))
    }

    pub fn year(self) -> u16 {
        (self.0 / 10000) as u16
    }
    pub fn month(self) -> u8 {
        (self.0 / 100 % 100) as u8
    }
    pub fn day(self) -> u8 {
        (self.0 % 100) as u8
    }

    /// Monday = 0, Sunday = 6. Uses the proleptic Gregorian calendar.
    pub fn weekday(self) -> usize {
        let offsets = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let y = self.year() as u32 - u32::from(self.month() < 3);
        ((y + y / 4 - y / 100
            + y / 400
            + offsets[self.month() as usize - 1]
            + self.day() as u32
            + 6)
            % 7) as usize
    }
}

impl FromStr for Date {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::InvalidDate);
        }
        let n: u32 = s.parse().map_err(|_| Error::InvalidDate)?;
        Self::new((n / 10000) as u16, (n / 100 % 100) as u8, (n % 100) as u8)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08}", self.0)
    }
}

/// Seconds since the start of the GTFS service day. `25:10:00` stays on that
/// service day and is never reduced modulo 24 hours. No timezone conversion is implied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Time(u32);

impl Time {
    pub fn seconds(self) -> u32 {
        self.0
    }
}

impl FromStr for Time {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        let mut parts = s.split(':');
        let h = parts.next().ok_or(Error::InvalidTime)?;
        let m = parts.next().ok_or(Error::InvalidTime)?;
        let sec = parts.next().ok_or(Error::InvalidTime)?;
        if parts.next().is_some()
            || h.is_empty()
            || m.len() != 2
            || sec.len() != 2
            || !h
                .bytes()
                .chain(m.bytes())
                .chain(sec.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return Err(Error::InvalidTime);
        }
        let h: u32 = h.parse().map_err(|_| Error::InvalidTime)?;
        let m: u32 = m.parse().map_err(|_| Error::InvalidTime)?;
        let sec: u32 = sec.parse().map_err(|_| Error::InvalidTime)?;
        if m >= 60 || sec >= 60 {
            return Err(Error::InvalidTime);
        }
        Ok(Self(
            h.checked_mul(3600)
                .and_then(|n| n.checked_add(m * 60 + sec))
                .ok_or(Error::InvalidTime)?,
        ))
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02}:{:02}:{:02}",
            self.0 / 3600,
            self.0 / 60 % 60,
            self.0 % 60
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Route<'a> {
    pub id: &'a str,
    pub agency_id: Option<&'a str>,
    pub short_name: &'a str,
    pub long_name: &'a str,
    pub description: Option<&'a str>,
    pub route_type: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Stop<'a> {
    pub id: &'a str,
    pub code: Option<&'a str>,
    pub name: Option<&'a str>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub parent_station: Option<&'a str>,
    pub location_type: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct Trip<'a> {
    pub id: &'a str,
    pub route_id: &'a str,
    pub service_id: &'a str,
    pub headsign: Option<&'a str>,
    pub direction: Option<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct StopTime<'a> {
    pub trip_id: &'a str,
    pub stop_id: &'a str,
    pub sequence: u32,
    pub arrival: Option<Time>,
    pub departure: Option<Time>,
    pub headsign: Option<&'a str>,
    pub pickup_type: u8,
    pub drop_off_type: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct Calendar<'a> {
    pub service_id: &'a str,
    pub weekdays: [bool; 7],
    pub start: Date,
    pub end: Date,
}

impl Calendar<'_> {
    pub fn runs_on(&self, date: Date) -> bool {
        self.start <= date && date <= self.end && self.weekdays[date.weekday()]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CalendarDate<'a> {
    pub service_id: &'a str,
    pub date: Date,
    pub added: bool,
}

impl<'a> Record<'a> {
    pub fn route(self) -> Result<Route<'a>, Error> {
        self.check(Table::Routes)?;
        Ok(Route {
            id: self.required(C::RouteId)?,
            agency_id: self.get(C::AgencyId),
            short_name: self.get(C::RouteShortName).unwrap_or(""),
            long_name: self.get(C::RouteLongName).unwrap_or(""),
            description: self.get(C::RouteDesc),
            route_type: self.number(C::RouteType)?,
        })
    }
    pub fn stop(self) -> Result<Stop<'a>, Error> {
        self.check(Table::Stops)?;
        let latitude = self.optional_number::<f64>(C::StopLat)?;
        let longitude = self.optional_number::<f64>(C::StopLon)?;
        if latitude.is_some_and(|v| !v.is_finite() || !(-90.0..=90.0).contains(&v)) {
            return Err(Error::InvalidField("stop_lat"));
        }
        if longitude.is_some_and(|v| !v.is_finite() || !(-180.0..=180.0).contains(&v)) {
            return Err(Error::InvalidField("stop_lon"));
        }
        Ok(Stop {
            id: self.required(C::StopId)?,
            code: self.get(C::StopCode),
            name: self.get(C::StopName),
            latitude,
            longitude,
            parent_station: self.get(C::ParentStation),
            location_type: self.optional_number(C::LocationType)?.unwrap_or(0),
        })
    }
    pub fn trip(self) -> Result<Trip<'a>, Error> {
        self.check(Table::Trips)?;
        let direction = self.optional_number(C::DirectionId)?;
        if direction.is_some_and(|n| n > 1) {
            return Err(Error::InvalidField("direction_id"));
        }
        Ok(Trip {
            id: self.required(C::TripId)?,
            route_id: self.required(C::RouteId)?,
            service_id: self.required(C::ServiceId)?,
            headsign: self.get(C::TripHeadsign),
            direction,
        })
    }
    pub fn stop_time(self) -> Result<StopTime<'a>, Error> {
        self.check(Table::StopTimes)?;
        let pickup_type = self.optional_number(C::PickupType)?.unwrap_or(0);
        let drop_off_type = self.optional_number(C::DropOffType)?.unwrap_or(0);
        if pickup_type > 3 {
            return Err(Error::InvalidField("pickup_type"));
        }
        if drop_off_type > 3 {
            return Err(Error::InvalidField("drop_off_type"));
        }
        Ok(StopTime {
            trip_id: self.required(C::TripId)?,
            stop_id: self.required(C::StopId)?,
            sequence: self.number(C::StopSequence)?,
            arrival: self.optional_number(C::ArrivalTime)?,
            departure: self.optional_number(C::DepartureTime)?,
            headsign: self.get(C::StopHeadsign),
            pickup_type,
            drop_off_type,
        })
    }
    pub fn calendar(self) -> Result<Calendar<'a>, Error> {
        self.check(Table::Calendar)?;
        let mut weekdays = [false; 7];
        for (i, c) in [
            C::Monday,
            C::Tuesday,
            C::Wednesday,
            C::Thursday,
            C::Friday,
            C::Saturday,
            C::Sunday,
        ]
        .into_iter()
        .enumerate()
        {
            weekdays[i] = match self.required(c)? {
                "0" => false,
                "1" => true,
                _ => return Err(Error::InvalidField(c.name())),
            };
        }
        let start = self.number(C::StartDate)?;
        let end = self.number(C::EndDate)?;
        if start > end {
            return Err(Error::InvalidField("end_date"));
        }
        Ok(Calendar {
            service_id: self.required(C::ServiceId)?,
            weekdays,
            start,
            end,
        })
    }
    pub fn calendar_date(self) -> Result<CalendarDate<'a>, Error> {
        self.check(Table::CalendarDates)?;
        let added = match self.required(C::ExceptionType)? {
            "1" => true,
            "2" => false,
            _ => return Err(Error::InvalidField("exception_type")),
        };
        Ok(CalendarDate {
            service_id: self.required(C::ServiceId)?,
            date: self.number(C::Date)?,
            added,
        })
    }
    fn check(self, table: Table) -> Result<(), Error> {
        if self.table == table {
            Ok(())
        } else {
            Err(Error::WrongTable)
        }
    }
    fn number<T: FromStr>(self, c: C) -> Result<T, Error> {
        self.required(c)?
            .parse()
            .map_err(|_| Error::InvalidField(c.name()))
    }
    fn optional_number<T: FromStr>(self, c: C) -> Result<Option<T>, Error> {
        self.get(c)
            .map(|s| s.parse().map_err(|_| Error::InvalidField(c.name())))
            .transpose()
    }
}
