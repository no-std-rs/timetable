//! Flash-backed IANA transitions and the legacy Europe/Warsaw clock.
use gtfs::Date as GtfsDate;
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};
pub fn valid_utc(utc: i64) -> bool {
    OffsetDateTime::from_unix_timestamp(utc).is_ok()
}

/// Selected IANA zone transitions held in immutable flash. Empty means legacy Warsaw.
#[derive(Clone, Copy)]
pub struct Zone<'a>(&'a [u8]);
impl<'a> Zone<'a> {
    pub const WARSAW: Self = Self(&[]);
    pub const fn from_bytes(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }
    fn i64(&self, n: usize) -> Option<i64> {
        Some(i64::from_le_bytes(self.0.get(n..n + 8)?.try_into().ok()?))
    }
    fn i32(&self, n: usize) -> Option<i32> {
        Some(i32::from_le_bytes(self.0.get(n..n + 4)?.try_into().ok()?))
    }
    pub fn valid(&self) -> bool {
        if self.0.is_empty() {
            return true;
        }
        let Some(start) = self.i64(0) else {
            return false;
        };
        let Some(end) = self.i64(8) else {
            return false;
        };
        let Some(initial) = self.i32(16) else {
            return false;
        };
        if start >= end
            || UtcOffset::from_whole_seconds(initial).is_err()
            || self.0.len() < 20
            || !(self.0.len() - 20).is_multiple_of(12)
            || self.0.len() > 20 + 32 * 12
        {
            return false;
        }
        let mut previous = start;
        for n in (20..self.0.len()).step_by(12) {
            let Some(at) = self.i64(n) else {
                return false;
            };
            let Some(offset) = self.i32(n + 8) else {
                return false;
            };
            if at <= previous || at >= end || UtcOffset::from_whole_seconds(offset).is_err() {
                return false;
            }
            previous = at;
        }
        true
    }
    pub fn local(self, utc: i64) -> Option<OffsetDateTime> {
        if self.0.is_empty() {
            return warsaw(utc);
        }
        if utc < self.i64(0)? || utc >= self.i64(8)? {
            return None;
        }
        let mut offset = self.i32(16)?;
        for n in (20..self.0.len()).step_by(12) {
            if utc < self.i64(n)? {
                break;
            }
            offset = self.i32(n + 8)?;
        }
        OffsetDateTime::from_unix_timestamp(utc)
            .ok()?
            .checked_to_offset(UtcOffset::from_whole_seconds(offset).ok()?)
    }
    pub fn service_date(self, utc: i64) -> Option<GtfsDate> {
        let d = self.local(utc)?;
        GtfsDate::new(d.year().try_into().ok()?, d.month() as u8, d.day()).ok()
    }
    pub fn service_epoch(self, date: GtfsDate) -> Option<i64> {
        if self.0.is_empty() {
            return service_epoch(date);
        }
        let d = Date::from_calendar_date(
            date.year().into(),
            Month::try_from(date.month()).ok()?,
            date.day(),
        )
        .ok()?;
        let noon = PrimitiveDateTime::new(d, Time::from_hms(12, 0, 0).ok()?);
        let naive = noon.assume_utc().unix_timestamp();
        let first = naive.checked_sub(i64::from(self.local(naive)?.offset().whole_seconds()))?;
        let actual = naive.checked_sub(i64::from(self.local(first)?.offset().whole_seconds()))?;
        let local = self.local(actual)?;
        (local.date() == d && local.time() == noon.time()).then(|| actual - 12 * 3600)
    }
}

pub fn warsaw(utc: i64) -> Option<OffsetDateTime> {
    let t = OffsetDateTime::from_unix_timestamp(utc).ok()?;
    let transition = |month: Month| {
        let last = Date::from_calendar_date(t.year(), month, 31).ok()?;
        let sunday = last.checked_sub(time::Duration::days(
            last.weekday().number_days_from_sunday().into(),
        ))?;
        Some(
            PrimitiveDateTime::new(sunday, Time::from_hms(1, 0, 0).ok()?)
                .assume_utc()
                .unix_timestamp(),
        )
    };
    let summer = (transition(Month::March)?..transition(Month::October)?).contains(&utc);
    Some(t.to_offset(UtcOffset::from_hms(if summer { 2 } else { 1 }, 0, 0).ok()?))
}

pub fn service_date(utc: i64) -> Option<GtfsDate> {
    let d = warsaw(utc)?;
    GtfsDate::new(d.year().try_into().ok()?, d.month() as u8, d.day()).ok()
}

/// GTFS times are measured from local noon minus twelve hours on the service day.
/// This also preserves the feed's 24:xx/25:xx times across midnight and DST.
pub fn service_epoch(date: GtfsDate) -> Option<i64> {
    let d = Date::from_calendar_date(
        date.year().into(),
        Month::try_from(date.month()).ok()?,
        date.day(),
    )
    .ok()?;
    let noon = PrimitiveDateTime::new(d, Time::from_hms(12, 0, 0).ok()?).assume_utc();
    let offset = warsaw(noon.unix_timestamp())?.offset().whole_seconds();
    Some(noon.unix_timestamp() - i64::from(offset) - 12 * 3600)
}

pub fn previous(date: GtfsDate) -> Option<GtfsDate> {
    adjacent(date, false)
}

pub fn next(date: GtfsDate) -> Option<GtfsDate> {
    adjacent(date, true)
}

fn adjacent(date: GtfsDate, forward: bool) -> Option<GtfsDate> {
    let d = Date::from_calendar_date(
        date.year().into(),
        Month::try_from(date.month()).ok()?,
        date.day(),
    )
    .ok()?;
    let d = if forward {
        d.next_day()?
    } else {
        d.previous_day()?
    };
    GtfsDate::new(d.year().try_into().ok()?, d.month() as u8, d.day()).ok()
}
