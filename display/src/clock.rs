//! Europe/Warsaw civil time, including EU daylight-saving transitions.
use gtfs::Date as GtfsDate;
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

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
    let d = Date::from_calendar_date(
        date.year().into(),
        Month::try_from(date.month()).ok()?,
        date.day(),
    )
    .ok()?
    .previous_day()?;
    GtfsDate::new(d.year().try_into().ok()?, d.month() as u8, d.day()).ok()
}
