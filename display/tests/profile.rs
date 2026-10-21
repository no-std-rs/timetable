use time::macros::datetime;
use timetable_display::{clock::Zone, profile::Profile};

fn rules(offset: i32, changes: &[(i64, i32)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(
        &datetime!(2024-01-01 0:00 UTC)
            .unix_timestamp()
            .to_le_bytes(),
    );
    b.extend_from_slice(
        &datetime!(2030-01-01 0:00 UTC)
            .unix_timestamp()
            .to_le_bytes(),
    );
    b.extend_from_slice(&offset.to_le_bytes());
    for (at, offset) in changes {
        b.extend_from_slice(&at.to_le_bytes());
        b.extend_from_slice(&offset.to_le_bytes());
    }
    b
}

#[test]
fn gtfs_noon_uses_the_selected_zone_across_dst_and_fractional_offsets() {
    let ny = rules(
        -18000,
        &[(datetime!(2026-03-08 7:00 UTC).unix_timestamp(), -14400)],
    );
    let zone = Zone::from_bytes(&ny);
    assert!(zone.valid());
    let day = "20260308".parse().unwrap();
    let epoch = zone.service_epoch(day).unwrap();
    assert_eq!(epoch, datetime!(2026-03-08 4:00 UTC).unix_timestamp());
    assert_eq!(zone.local(epoch + 12 * 3600).unwrap().hour(), 12);
    assert_eq!(
        zone.service_date(datetime!(2026-03-08 2:00 UTC).unix_timestamp())
            .unwrap(),
        "20260307".parse().unwrap()
    );
    let india = rules(19800, &[]);
    let zone = Zone::from_bytes(&india);
    assert_eq!(
        zone.service_epoch(day),
        Some(datetime!(2026-03-07 18:30 UTC).unix_timestamp())
    );
    assert!(
        zone.local(datetime!(2030-01-01 0:00 UTC).unix_timestamp())
            .is_none()
    );
    // A transition can occur between UTC noon and the actual local noon.
    let east = rules(
        12 * 3600,
        &[(datetime!(2026-04-04 15:00 UTC).unix_timestamp(), 13 * 3600)],
    );
    let zone = Zone::from_bytes(&east);
    let day = "20260405".parse().unwrap();
    assert_eq!(
        zone.service_epoch(day),
        Some(datetime!(2026-04-04 11:00 UTC).unix_timestamp())
    );
    // Non-hour southern-hemisphere changes must retain GTFS elapsed seconds.
    let island = rules(
        11 * 3600,
        &[(datetime!(2026-04-04 15:00 UTC).unix_timestamp(), 37800)],
    );
    assert_eq!(
        Zone::from_bytes(&island).service_epoch(day),
        Some(datetime!(2026-04-04 13:30 UTC).unix_timestamp())
    );
}

#[test]
fn checked_profile_binds_rules_and_id_but_not_glyphs() {
    let zone = rules(19800, &[]);
    let mut b = vec![0; 250];
    b[..8].copy_from_slice(b"TTPRF001");
    b[8..14].copy_from_slice(b"tdg-42");
    b[88..100].copy_from_slice(b"Asia/Kolkata");
    b[152..156].copy_from_slice(b"Test");
    b[248..250].copy_from_slice(&(zone.len() as u16).to_le_bytes());
    b.extend_from_slice(&zone);
    let identity = Profile::new(&b).unwrap().identity().to_vec();
    b.extend_from_slice(b"TTGLY001\x01\x00");
    b.extend_from_slice(&('東' as u32).to_le_bytes());
    for _ in 0..10 {
        b.extend_from_slice(&1023u16.to_le_bytes());
    }
    let p = Profile::new(&b).unwrap();
    assert_eq!(p.feed_id(), Some("tdg-42"));
    assert_eq!(p.identity(), identity);
    assert_eq!(p.glyph('東').unwrap().len(), 20);
    assert!(p.glyph('西').is_none());
    let mut invalid = b.clone();
    *invalid.last_mut().unwrap() = 255;
    assert!(Profile::new(&invalid).is_none());
    b[8] = b'/';
    assert!(Profile::new(&b).is_none());
}
