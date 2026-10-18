//! Successful check time and source-provided modification time, in UTC.
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Freshness {
    pub checked_at: Option<i64>,
    /// Oldest source timestamp used: HTTP Last-Modified, falling back to the
    /// latest UTC GTFS table mtime in that ZIP. Unknown if any source lacks both;
    /// never substitute a service date or HTTP Date header.
    pub source_updated_at: Option<i64>,
}

pub struct SourceTimes {
    oldest: Option<i64>,
    complete: bool,
}
impl Default for SourceTimes {
    fn default() -> Self {
        Self {
            oldest: None,
            complete: true,
        }
    }
}
impl SourceTimes {
    pub fn observe(&mut self, modified: Option<i64>) {
        self.complete &= modified.is_some();
        if let Some(stamp) = modified {
            self.oldest = Some(self.oldest.map_or(stamp, |old| old.min(stamp)));
        }
    }
    pub fn updated_at(&self) -> Option<i64> {
        self.oldest.filter(|_| self.complete)
    }
}

/// Optional source metadata; duplicate fields and dates in the future are
/// unknown. HTTP Date is the response time and must never stand in for this.
pub fn source_modified<'a>(
    headers: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    now: i64,
) -> Option<i64> {
    let mut seen = false;
    let mut modified = None;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("last-modified") {
            if seen {
                return None;
            }
            seen = true;
            modified = http_date(value, now).filter(|stamp| *stamp <= now);
        }
    }
    modified
}

/// RFC 9110 HTTP-date, including both obsolete recipient formats. Bounded
/// stack, no allocation or parsing-feature dependency. `now` resolves RFC850 years.
pub fn http_date(value: &[u8], now: i64) -> Option<i64> {
    let value = core::str::from_utf8(value).ok()?;
    let mut fields = [""; 6];
    let mut count = 0;
    for field in value.split_ascii_whitespace() {
        *fields.get_mut(count)? = field;
        count += 1;
    }
    let number = |s: &str| -> Option<u32> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let (weekday, day, month, year, clock, full_weekday) = match count {
        6 if fields[0].len() == 4
            && fields[0].ends_with(',')
            && fields[1].len() == 2
            && fields[3].len() == 4
            && fields[5] == "GMT" =>
        {
            (
                &fields[0][..3],
                fields[1],
                fields[2],
                number(fields[3])? as i32,
                fields[4],
                false,
            )
        }
        4 if fields[0].ends_with(',') && fields[1].len() == 9 && fields[3] == "GMT" => {
            let date = fields[1];
            if date.get(2..3)? != "-" || date.get(6..7)? != "-" {
                return None;
            }
            let reference = OffsetDateTime::from_unix_timestamp(now).ok()?.year();
            let mut year = reference.div_euclid(100) * 100 + number(date.get(7..9)?)? as i32;
            if year > reference + 50 {
                year -= 100;
            }
            (
                &fields[0][..fields[0].len() - 1],
                date.get(..2)?,
                date.get(3..6)?,
                year,
                fields[2],
                true,
            )
        }
        5 if fields[0].len() == 3 && (1..=2).contains(&fields[2].len()) && fields[4].len() == 4 => {
            (
                fields[0],
                fields[2],
                fields[1],
                number(fields[4])? as i32,
                fields[3],
                false,
            )
        }
        _ => return None,
    };
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == month)?
        + 1;
    if clock.len() != 8 || clock.get(2..3)? != ":" || clock.get(5..6)? != ":" {
        return None;
    }
    let second = number(clock.get(6..8)?)?;
    if second > 60 {
        return None;
    }
    let time = Time::from_hms(
        number(clock.get(..2)?)?.try_into().ok()?,
        number(clock.get(3..5)?)?.try_into().ok()?,
        second.min(59) as u8,
    )
    .ok()?;
    let day = number(day)?.try_into().ok()?;
    let mut year = year;
    if full_weekday {
        let reference = OffsetDateTime::from_unix_timestamp(now).ok()?;
        if year == reference.year() + 50
            && (month as u8, day, time.hour(), time.minute(), second)
                > (
                    reference.month() as u8,
                    reference.day(),
                    reference.hour(),
                    reference.minute(),
                    u32::from(reference.second()),
                )
        {
            year -= 100;
        }
    }
    let date = Date::from_calendar_date(year, Month::try_from(month as u8).ok()?, day).ok()?;
    let weekdays = if full_weekday {
        [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ]
    } else {
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    };
    if weekday != weekdays[date.weekday().number_days_from_monday() as usize] {
        return None;
    }
    PrimitiveDateTime::new(date, time)
        .assume_utc()
        .unix_timestamp()
        .checked_add(i64::from(second == 60))
}
