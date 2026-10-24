//! Versioned, checksummed snapshot tied to the exact provisioned settings.
use timetable_display::{App, Departure, departures::Departures, model::Headings};
const HEADER: usize = 48;
pub fn encode(settings: &[u8], app: &App, output: &mut [u8]) -> Result<usize, &'static str> {
    if output.len() < HEADER || app.loaded_at.is_none() {
        return Err("Invalid snapshot");
    }
    output[..HEADER].fill(0);
    output[..8].copy_from_slice(b"TTSNAP01");
    output[12..16].copy_from_slice(&timetable_display::config::crc32(settings).to_le_bytes());
    let epoch = app.loaded_at.unwrap();
    output[16..24].copy_from_slice(&epoch.to_le_bytes());
    output[24..32].copy_from_slice(
        &app.freshness
            .source_updated_at
            .unwrap_or(i64::MIN)
            .to_le_bytes(),
    );
    output[40..48].copy_from_slice(&app.freshness.checked_at.unwrap_or(i64::MIN).to_le_bytes());
    let mut at = HEADER;
    let mut count = 0;
    while let Some(name) = app.headsigns.get(count) {
        let end = at + 1 + name.len();
        let bytes = output.get_mut(at..end).ok_or("Snapshot buffer too small")?;
        bytes[0] = name.len() as u8;
        bytes[1..].copy_from_slice(name.as_bytes());
        at = end;
        count += 1;
    }
    output[32] = count;
    output[34..36].copy_from_slice(&(app.departures.len() as u16).to_le_bytes());
    for event in app.departures.iter() {
        let row = output
            .get_mut(at..at + 12)
            .ok_or("Snapshot buffer too small")?;
        row[..8].copy_from_slice(&event.at.to_le_bytes());
        row[8..].copy_from_slice(&[
            event.group,
            event.pick,
            u8::from(event.conditional),
            event.headsign,
        ]);
        at += 12;
    }
    if app.journeys.until > app.journeys.epoch {
        output[33] = 1;
        at += app
            .journeys
            .encode(output.get_mut(at..).ok_or("Snapshot buffer too small")?)?;
    }
    let total = at + 4;
    output
        .get_mut(at..total)
        .ok_or("Snapshot buffer too small")?;
    output[8..12].copy_from_slice(&(total as u32).to_le_bytes());
    let checksum = timetable_display::config::crc32(&output[..at]);
    output[at..total].copy_from_slice(&checksum.to_le_bytes());
    Ok(total)
}
pub fn restore(settings: &[u8], input: &[u8], app: &mut App) -> Result<(), &'static str> {
    let invalid = "Saved timetable is invalid or belongs to different settings";
    if input.len() < HEADER + 4 || &input[..8] != b"TTSNAP01" {
        return Err(invalid);
    }
    let total = u32::from_le_bytes(input[8..12].try_into().unwrap()) as usize;
    let input = input
        .get(..total)
        .filter(|b| b.len() >= HEADER + 4)
        .ok_or(invalid)?;
    let end = total - 4;
    if timetable_display::config::crc32(&input[..end])
        != u32::from_le_bytes(input[end..].try_into().unwrap())
        || timetable_display::config::crc32(settings)
            != u32::from_le_bytes(input[12..16].try_into().unwrap())
    {
        return Err(invalid);
    }
    let settings_value = crate::Settings::decode(settings)?;
    let config = settings_value.config();
    let epoch = i64::from_le_bytes(input[16..24].try_into().unwrap());
    if settings_value.timezone().local(epoch).is_none() {
        return Err(invalid);
    }
    let source = i64::from_le_bytes(input[24..32].try_into().unwrap());
    let mut events = Departures::new(epoch);
    let mut headings = Headings::default();
    let mut at = HEADER;
    if usize::from(input[32]) > timetable_display::model::MAX_HEADSIGNS {
        return Err(invalid);
    }
    for index in 0..input[32] {
        let len = usize::from(*input.get(at).ok_or(invalid)?);
        at += 1;
        let name =
            core::str::from_utf8(input.get(at..at + len).ok_or(invalid)?).map_err(|_| invalid)?;
        if name.len() > 64 || name.chars().any(char::is_control) {
            return Err(invalid);
        }
        if headings.insert(name).map_err(|_| invalid)? != index {
            return Err(invalid);
        }
        at += len;
    }
    let count = u16::from_le_bytes(input[34..36].try_into().unwrap());
    for _ in 0..count {
        let row = input
            .get(at..at + 12)
            .filter(|_| at + 12 <= end)
            .ok_or(invalid)?;
        let group = row[8];
        let pick = row[9];
        if config
            .groups
            .get(usize::from(group))
            .and_then(|g| g.picks.get(usize::from(pick)))
            .is_none()
            || row[10] > 1
            || headings.get(row[11]).is_none()
        {
            return Err(invalid);
        }
        events
            .push(Departure {
                at: i64::from_le_bytes(row[..8].try_into().unwrap()),
                group,
                pick,
                conditional: row[10] == 1,
                headsign: row[11],
            })
            .map_err(|_| invalid)?;
        at += 12;
    }
    let journeys = if input[33] == 1 {
        timetable_display::journey::Cache::decode(
            input.get(at..end).ok_or(invalid)?,
            &config,
            &headings,
        )?
    } else if input[33] == 0 && at == end {
        timetable_display::journey::Cache::default()
    } else {
        return Err(invalid);
    };
    app.publish(events, headings, epoch);
    app.journeys = journeys;
    let checked = i64::from_le_bytes(input[40..48].try_into().unwrap());
    app.freshness.checked_at = (checked != i64::MIN).then_some(checked);
    app.freshness.source_updated_at = (source != i64::MIN).then_some(source);
    Ok(())
}
