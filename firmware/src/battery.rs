//! Bounded RTC snapshot and duty-cycle policy. Never retain credentials.
use timetable_display::{
    App, Config, Departure, config,
    departures::{Departures, HORIZON_SECONDS},
    model::{Headings, Status},
};

pub const IDLE_SECONDS: u64 = 30;
pub const REFRESH_SECONDS: u64 = 3 * 3600;
pub const RETRY_SECONDS: u64 = 5 * 60;
pub const RETAINED_BYTES: usize = 4096;
const HEADER: usize = 96;
const MAGIC: &[u8; 8] = b"TTSLEEP2";
const UNKNOWN_TIME: i64 = i64::MIN;

fn valid_freshness(
    loaded: Option<i64>,
    freshness: timetable_display::freshness::Freshness,
) -> bool {
    !freshness.checked_at.is_some_and(|t| {
        timetable_display::clock::warsaw(t).is_none() || loaded.is_none_or(|epoch| t < epoch)
    }) && !freshness.source_updated_at.is_some_and(|t| {
        timetable_display::clock::warsaw(t).is_none()
            || freshness.checked_at.is_none_or(|check| t > check)
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Capacity,
}

#[derive(Debug)]
pub struct Wake {
    pub utc_micros: Option<i64>,
    pub refresh_in_us: u64,
    pub show_screen: bool,
}

pub struct WakeUi {
    pub visible: bool,
    pub suppress_touch: bool,
}

pub fn wake_ui(restored: Option<&Wake>, timer: bool, touch: bool) -> WakeUi {
    let suppress_touch = touch || restored.is_some_and(|wake| wake.show_screen);
    WakeUi {
        visible: restored.is_none() || !timer || suppress_touch,
        suppress_touch,
    }
}

/// A touch during radio shutdown is presented on the immediate timer reboot.
pub fn show_on_wake(image: &mut [u8; RETAINED_BYTES]) {
    image[69] = 1;
    image[8..12].fill(0);
    let crc = config::crc32(image);
    image[8..12].copy_from_slice(&crc.to_le_bytes());
}

pub fn refresh_due(
    radio_available: bool,
    shutting_down: bool,
    now_us: u64,
    deadline_us: u64,
) -> bool {
    radio_available && !shutting_down && now_us >= deadline_us
}

pub fn can_sleep(app: &App, busy: bool, idle_seconds: u64, touching: bool) -> bool {
    app.screen == timetable_display::Screen::Board
        && app.draft.is_none()
        && !app.save_failed
        && !busy
        && !touching
        && idle_seconds >= IDLE_SECONDS
}

pub fn next_refresh(rtc_us: u64, success: bool) -> u64 {
    rtc_us.saturating_add(
        if success {
            REFRESH_SECONDS
        } else {
            RETRY_SECONDS
        } * 1_000_000,
    )
}

/// Encode only a completed app snapshot. Caller excludes active queries and drafts.
pub fn save(
    image: &mut [u8; RETAINED_BYTES],
    app: &App,
    config_tag: u32,
    rtc_ticks: u64,
    tick_period: u32,
    utc_micros: Option<i64>,
    refresh_in_us: u64,
) -> Result<(), Error> {
    image.fill(0);
    if tick_period == 0
        || refresh_in_us > REFRESH_SECONDS * 1_000_000
        || !valid_freshness(app.loaded_at, app.freshness)
    {
        return Err(Error::Invalid);
    }
    let epoch = app.loaded_at.or(app.clock).unwrap_or(0);
    image[12..16].copy_from_slice(&config_tag.to_le_bytes());
    image[16..24].copy_from_slice(&rtc_ticks.to_le_bytes());
    image[72..76].copy_from_slice(&tick_period.to_le_bytes());
    image[24..32].copy_from_slice(&utc_micros.unwrap_or(UNKNOWN_TIME).to_le_bytes());
    image[32..40].copy_from_slice(&refresh_in_us.to_le_bytes());
    image[40..48].copy_from_slice(&app.loaded_at.unwrap_or(UNKNOWN_TIME).to_le_bytes());
    image[80..88].copy_from_slice(
        &app.freshness
            .checked_at
            .unwrap_or(UNKNOWN_TIME)
            .to_le_bytes(),
    );
    image[88..96].copy_from_slice(
        &app.freshness
            .source_updated_at
            .unwrap_or(UNKNOWN_TIME)
            .to_le_bytes(),
    );
    image[48..56].copy_from_slice(&app.manual_until.to_le_bytes());
    image[56..64].copy_from_slice(&epoch.to_le_bytes());
    image[64] = app.group.try_into().map_err(|_| Error::Invalid)?;
    image[65] = match app.status {
        Status::Ready => 0,
        Status::Offline => 1,
        Status::Failed => 2,
        Status::Capacity => 3,
        _ => return Err(Error::Invalid),
    };
    image[66..68].copy_from_slice(&(app.departures.len() as u16).to_le_bytes());
    let mut end = HEADER;
    for event in app.departures.iter() {
        let seconds = event.at.checked_sub(epoch).ok_or(Error::Invalid)?;
        if !(1..=HORIZON_SECONDS).contains(&seconds)
            || event.group >= 4
            || event.pick >= 4
            || app.headsigns.get(event.headsign).is_none()
        {
            return Err(Error::Invalid);
        }
        let slot = image.get_mut(end..end + 4).ok_or(Error::Capacity)?;
        slot[..2].copy_from_slice(&(seconds as u16).to_le_bytes());
        slot[2] = event.group * 4 + event.pick + u8::from(event.conditional) * 16;
        slot[3] = event.headsign;
        end += 4;
    }
    for i in 0..32 {
        let Some(name) = app.headsigns.get(i) else {
            break;
        };
        let slot = image
            .get_mut(end..end + 1 + name.len())
            .ok_or(Error::Capacity)?;
        slot[0] = name.len() as u8;
        slot[1..].copy_from_slice(name.as_bytes());
        end += slot.len();
        image[68] += 1;
    }
    image[70..72].copy_from_slice(&(end as u16).to_le_bytes());
    image[..8].copy_from_slice(MAGIC);
    let crc = config::crc32(image);
    image[8..12].copy_from_slice(&crc.to_le_bytes());
    Ok(())
}

/// Validate all bytes and stage decoded data before changing the visible app.
pub fn restore(
    image: &mut [u8; RETAINED_BYTES],
    settings: &Config,
    config_tag: u32,
    rtc_ticks: u64,
    app: &mut App,
) -> Result<Wake, Error> {
    if &image[..8] != MAGIC || image[12..16] != config_tag.to_le_bytes() {
        return Err(Error::Invalid);
    }
    let expected = u32::from_le_bytes(image[8..12].try_into().unwrap());
    image[8..12].fill(0);
    let actual = config::crc32(image);
    image[8..12].copy_from_slice(&expected.to_le_bytes());
    if actual != expected {
        return Err(Error::Invalid);
    }
    let u64_at = |at| u64::from_le_bytes(image[at..at + 8].try_into().unwrap());
    let i64_at = |at| i64::from_le_bytes(image[at..at + 8].try_into().unwrap());
    // HAL recalibrates on boot: scale the raw tick *difference* with the saved
    // period, never subtract absolute microsecond values from different boots.
    let period = u32::from_le_bytes(image[72..76].try_into().unwrap());
    if period == 0 {
        return Err(Error::Invalid);
    }
    let elapsed = rtc_ticks
        .checked_sub(u64_at(16))
        .and_then(|ticks| ticks.checked_mul(u64::from(period)))
        .map(|scaled| scaled >> 19)
        .filter(|us| *us <= 7 * 86400 * 1_000_000)
        .ok_or(Error::Invalid)?;
    let utc_micros = (i64_at(24) != UNKNOWN_TIME)
        .then(|| i64_at(24).checked_add(elapsed as i64))
        .flatten();
    if i64_at(24) != UNKNOWN_TIME && utc_micros.is_none() {
        return Err(Error::Invalid);
    }
    let now = utc_micros.map(|us| us.div_euclid(1_000_000));
    if now.is_some_and(|now| timetable_display::clock::warsaw(now).is_none()) {
        return Err(Error::Invalid);
    }
    let refresh_in_us = u64_at(32).saturating_sub(elapsed);
    if u64_at(32) > REFRESH_SECONDS * 1_000_000 {
        return Err(Error::Invalid);
    }
    let loaded_at = (i64_at(40) != UNKNOWN_TIME).then(|| i64_at(40));
    let checked_at = (i64_at(80) != UNKNOWN_TIME).then(|| i64_at(80));
    let source_updated_at = (i64_at(88) != UNKNOWN_TIME).then(|| i64_at(88));
    if !valid_freshness(
        loaded_at,
        timetable_display::freshness::Freshness {
            checked_at,
            source_updated_at,
        },
    ) {
        return Err(Error::Invalid);
    }
    let manual_until = i64_at(48);
    let epoch = i64_at(56);
    let group = usize::from(image[64]);
    let status = match image[65] {
        0 => Status::Ready,
        1 => Status::Offline,
        2 => Status::Failed,
        3 => Status::Capacity,
        _ => return Err(Error::Invalid),
    };
    let count = usize::from(u16::from_le_bytes(image[66..68].try_into().unwrap()));
    let headings = image[68];
    let used = usize::from(u16::from_le_bytes(image[70..72].try_into().unwrap()));
    if image[69] > 1
        || group >= settings.groups.len()
        || count > 512
        || headings > 32
        || used > RETAINED_BYTES
        || HEADER + count * 4 > used
    {
        return Err(Error::Invalid);
    }
    let mut events = Departures::new(epoch);
    let mut names = Headings::default();
    let mut end = HEADER;
    // Saved absolute times are useful before NTP too. Never promote the saved
    // query reference to UTC; the renderer keeps countdowns disabled.
    if count > 0 && loaded_at != Some(epoch) {
        return Err(Error::Invalid);
    }
    for _ in 0..count {
        let slot = &image[end..end + 4];
        let selection = slot[2];
        let group = usize::from((selection >> 2) & 3);
        let pick = usize::from(selection & 3);
        if selection > 31
            || slot[3] >= headings
            || settings
                .groups
                .get(group)
                .and_then(|g| g.picks.get(pick))
                .is_none()
        {
            return Err(Error::Invalid);
        }
        events
            .push(Departure {
                at: epoch
                    .checked_add(i64::from(u16::from_le_bytes(slot[..2].try_into().unwrap())))
                    .ok_or(Error::Invalid)?,
                group: group as u8,
                pick: pick as u8,
                conditional: selection & 16 != 0,
                headsign: slot[3],
            })
            .map_err(|_| Error::Invalid)?;
        end += 4;
    }
    for index in 0..headings {
        let len = usize::from(
            *image
                .get(end)
                .filter(|_| end < used)
                .ok_or(Error::Invalid)?,
        );
        let bytes = image
            .get(end + 1..end + 1 + len)
            .filter(|_| end + 1 + len <= used && len <= 64)
            .ok_or(Error::Invalid)?;
        let name = core::str::from_utf8(bytes).map_err(|_| Error::Invalid)?;
        if names.insert(name).map_err(|_| Error::Invalid)? != index {
            return Err(Error::Invalid);
        }
        end += 1 + len;
    }
    if end != used {
        return Err(Error::Invalid);
    }
    app.publish(events, names, epoch);
    app.clock = now;
    app.loaded_at = loaded_at;
    app.freshness = timetable_display::freshness::Freshness {
        checked_at,
        source_updated_at,
    };
    app.group = group;
    app.manual_until = manual_until;
    app.status = status;
    Ok(Wake {
        utc_micros,
        refresh_in_us,
        show_screen: image[69] != 0,
    })
}
