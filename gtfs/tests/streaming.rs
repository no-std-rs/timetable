use embedded_io_async::{ErrorType, Read, Seek, SeekFrom};
use futures::executor::block_on;
use gtfs::{
    Date, Error, Feed, Id, IdSet, Table, Time, TripIndex, TripSlot, csv, http, poznan, stream, zip,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/gtfs.zip");
struct Memory<'a> {
    data: &'a [u8],
    pos: u64,
    chunk: usize,
}
impl<'a> Memory<'a> {
    fn new(data: &'a [u8], chunk: usize) -> Self {
        Self {
            data,
            pos: 0,
            chunk,
        }
    }
}
impl ErrorType for Memory<'_> {
    type Error = Error;
}
impl Read for Memory<'_> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        let start = (self.pos as usize).min(self.data.len());
        let n = out.len().min(self.chunk).min(self.data.len() - start);
        out[..n].copy_from_slice(&self.data[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}
impl Seek for Memory<'_> {
    async fn seek(&mut self, p: SeekFrom) -> Result<u64, Error> {
        self.pos = match p {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => self.pos.checked_add_signed(n).ok_or(Error::InvalidRange)?,
            SeekFrom::End(n) => (self.data.len() as u64)
                .checked_add_signed(n)
                .ok_or(Error::InvalidRange)?,
        };
        Ok(self.pos)
    }
}

/// Intentionally has no Seek implementation; streaming APIs cannot seek here.
struct Forward<R>(R);
impl<R: ErrorType> ErrorType for Forward<R> {
    type Error = R::Error;
}
impl<R: Read> Read for Forward<R> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(out).await
    }
}

#[test]
fn cache_verification_checks_all_payloads_before_accepting_the_archive() {
    block_on(async {
        let mut workspace = zip::Workspace::new();
        let mut entries = [zip::VerificationSlot::EMPTY; 8];
        for data in [
            FIXTURE,
            include_bytes!("fixtures/descriptor.zip").as_slice(),
            include_bytes!("fixtures/stored.zip").as_slice(),
            include_bytes!("fixtures/comment.zip").as_slice(),
        ] {
            let directory = zip::Directory::read(&mut Memory::new(data, 3), &mut [0; 64])
                .await
                .unwrap();
            let manifest = zip::verify(
                Forward(Memory::new(data, 1)),
                &mut workspace,
                &mut [0; 17],
                &mut entries,
            )
            .await
            .unwrap();
            assert_eq!(manifest, zip::Manifest::from_directory(&directory));
            assert!(
                zip::verify(
                    Forward(Memory::new(&data[..data.len() - 1], 3)),
                    &mut workspace,
                    &mut [0; 17],
                    &mut entries,
                )
                .await
                .is_err()
            );
        }
        for name in [b"routes.txt", b"shapes.txt"] {
            let mut corrupt = include_bytes!("fixtures/stored.zip").to_vec();
            let offsets: Vec<_> = corrupt
                .windows(10)
                .enumerate()
                .filter_map(|(i, bytes)| (bytes == b"routes.txt").then_some(i))
                .collect();
            for offset in offsets {
                corrupt[offset..offset + 10].copy_from_slice(name);
            }
            let name_len = u16::from_le_bytes([corrupt[26], corrupt[27]]) as usize;
            let extra_len = u16::from_le_bytes([corrupt[28], corrupt[29]]) as usize;
            corrupt[30 + name_len + extra_len] ^= 1;
            // Directory-only validation accepts the intact headers and footer.
            zip::Directory::read(&mut Memory::new(&corrupt, 3), &mut [0; 64])
                .await
                .unwrap();
            assert_eq!(
                zip::verify(
                    Forward(Memory::new(&corrupt, 1)),
                    &mut workspace,
                    &mut [0; 17],
                    &mut entries,
                )
                .await,
                Err(Error::CrcMismatch)
            );
        }
        assert_eq!(
            zip::verify(
                Forward(Memory::new(FIXTURE, 1)),
                &mut workspace,
                &mut [],
                &mut entries
            )
            .await,
            Err(Error::BufferTooSmall)
        );
        assert_eq!(
            zip::verify(
                Forward(Memory::new(FIXTURE, 1)),
                &mut workspace,
                &mut [0; 17],
                &mut [zip::VerificationSlot::EMPTY; 1],
            )
            .await,
            Err(Error::IndexFull)
        );
    });
}

#[test]
fn cache_verification_matches_all_local_and_directory_metadata() {
    block_on(async {
        let end = FIXTURE.len() - 22;
        let directory_start =
            u32::from_le_bytes(FIXTURE[end + 16..end + 20].try_into().unwrap()) as usize;
        let mut records = Vec::new();
        let mut pos = directory_start;
        while pos < end {
            let len = |i| u16::from_le_bytes([FIXTURE[pos + i], FIXTURE[pos + i + 1]]) as usize;
            let size = 46 + len(28) + len(30) + len(32);
            records.push((pos, size, &FIXTURE[pos + 46..pos + 46 + len(28)]));
            pos += size;
        }
        let mut workspace = zip::Workspace::new();
        let mut slots = [zip::VerificationSlot::EMPTY; 8];
        for name in [b"routes.txt", b"shapes.txt"] {
            let offset = records.iter().find(|record| record.2 == name).unwrap().0;
            // Flags, method, CRC, compressed/uncompressed size, offset and name.
            for field in [8, 10, 16, 20, 24, 42, 46] {
                let mut corrupt = FIXTURE.to_vec();
                match field {
                    8 => corrupt[offset + field] ^= 8, // Supported descriptor flag.
                    10 => corrupt[offset + field] = 0, // Supported stored method.
                    _ => corrupt[offset + field] ^= 1,
                }
                assert!(
                    zip::verify(
                        Forward(Memory::new(&corrupt, 3)),
                        &mut workspace,
                        &mut [0; 17],
                        &mut slots,
                    )
                    .await
                    .is_err(),
                    "accepted mismatched {name:?} field {field}"
                );
            }
        }
        // Directory order is independent of physical local-entry order.
        let mut reordered = FIXTURE[..directory_start].to_vec();
        for (offset, size, _) in records.iter().rev() {
            reordered.extend_from_slice(&FIXTURE[*offset..*offset + *size]);
        }
        reordered.extend_from_slice(&FIXTURE[end..]);
        let expected = zip::verify(
            Forward(Memory::new(FIXTURE, 3)),
            &mut workspace,
            &mut [0; 17],
            &mut slots,
        )
        .await
        .unwrap();
        assert_eq!(
            zip::verify(
                Forward(Memory::new(&reordered, 1)),
                &mut workspace,
                &mut [0; 17],
                &mut slots,
            )
            .await
            .unwrap(),
            expected
        );
    });
}

#[test]
fn directory_search_skips_false_end_records_inside_valid_comments() {
    block_on(async {
        let end = FIXTURE.len() - 22;
        let prefix = b"comment-prefix";
        let suffix = b"comment-suffix";
        for kind in 0..3 {
            let mut data = FIXTURE.to_vec();
            let comment_len = (prefix.len() + 22 + suffix.len()) as u16;
            data[end + 20..end + 22].copy_from_slice(&comment_len.to_le_bytes());
            data.extend_from_slice(prefix);
            let fake_offset = data.len();
            let mut fake = [0u8; 22];
            fake[..4].copy_from_slice(b"PK\x05\x06");
            fake[20..22].copy_from_slice(&(suffix.len() as u16).to_le_bytes());
            match kind {
                0 => fake[4] = 1, // Unsupported disk in a false candidate.
                1 => {}           // Supported fields, inconsistent directory bounds.
                _ => {
                    // Plausible bounds and real entries, but the fake directory
                    // includes the genuine end record and a comment prefix.
                    fake[8..12].copy_from_slice(&FIXTURE[end + 8..end + 12]);
                    fake[16..20].copy_from_slice(&FIXTURE[end + 16..end + 20]);
                    let start = u32::from_le_bytes(fake[16..20].try_into().unwrap());
                    let size = fake_offset as u32 - start;
                    fake[12..16].copy_from_slice(&size.to_le_bytes());
                }
            }
            data.extend_from_slice(&fake);
            data.extend_from_slice(suffix);
            for capacity in [22, 23, 64, 1024] {
                let mut scratch = vec![0; capacity];
                let directory = zip::Directory::read(&mut Memory::new(&data, 1), &mut scratch)
                    .await
                    .unwrap();
                assert!(directory.member(Table::Routes).is_some());
            }
        }
    });
}

#[test]
fn marker_valued_crc_accepts_both_descriptor_layouts() {
    block_on(async {
        for data in [
            include_bytes!("fixtures/marker-crc-signed.zip").as_slice(),
            include_bytes!("fixtures/marker-crc-unsigned.zip").as_slice(),
        ] {
            let mut workspace = zip::Workspace::new();
            let mut reader = zip::Reader::new(Forward(Memory::new(data, 1)), &mut workspace);
            assert_eq!(
                reader.next_entry().await.unwrap().unwrap().name(),
                b"rare-crc.bin"
            );
            let mut output = Vec::new();
            let mut scratch = [0; 3];
            loop {
                let n = reader.read(&mut scratch).await.unwrap();
                if n == 0 {
                    break;
                }
                output.extend_from_slice(&scratch[..n]);
            }
            assert_eq!(output, [0xac, 0x0a, 0x7a, 0xd5]);
            assert_eq!(crc32fast::hash(&output), 0x08074b50);
            assert!(reader.next_entry().await.unwrap().is_none());
            reader.finish_archive(&zip::Manifest::EMPTY).await.unwrap();
            zip::verify(
                Forward(Memory::new(data, 1)),
                &mut workspace,
                &mut [0; 3],
                &mut [zip::VerificationSlot::EMPTY; 1],
            )
            .await
            .unwrap();
        }
    });
}

#[test]
fn official_queries_need_only_read_and_accept_arbitrary_member_order() {
    block_on(async {
        for data in [
            FIXTURE,
            include_bytes!("fixtures/descriptor.zip").as_slice(),
            include_bytes!("fixtures/shuffled.zip").as_slice(),
        ] {
            let mut trip_slots = [TripSlot::<32>::EMPTY; 8];
            let mut trips = TripIndex::new(&mut trip_slots);
            let mut service_slots = [stream::ServiceSlot::<32>::EMPTY; 8];
            let mut services = stream::Services::new(&mut service_slots);
            let mut workspace = zip::Workspace::new();
            let manifest = stream::prepare(
                Forward(Memory::new(data, 3)),
                stream::Selection {
                    routes: &["4"],
                    direction: None,
                    date: Some("20261017".parse().unwrap()),
                },
                &mut trips,
                &mut services,
                &mut workspace,
            )
            .await
            .unwrap();
            assert_eq!(trips.len(), 3);
            let mut events = Vec::new();
            stream::departures(
                Forward(Memory::new(data, 1)),
                &manifest,
                &trips,
                &["S1"],
                &mut workspace,
                |d| {
                    events.push(d.event.trip_id.to_owned());
                    Ok(())
                },
            )
            .await
            .unwrap();
            assert_eq!(events, ["T_A", "T_B", "T_C"]);
        }
    });
}

#[test]
fn mirror_manifest_matches_repacked_original_content_and_stale_metadata_fails() {
    block_on(async {
        let feed = Feed::open(Memory::new(FIXTURE, 64), &mut [0; 128])
            .await
            .unwrap();
        let manifest = zip::Manifest::from_directory(feed.directory());
        let mut trip_slots = [TripSlot::<32>::EMPTY; 1];
        let mut trips = TripIndex::new(&mut trip_slots);
        trips.insert("T_A", 0, Some(0)).unwrap();
        trips.finish().unwrap();
        let mut work = zip::Workspace::new();
        let mut events = 0;
        // Different member ordering/compression does not change table identity.
        stream::departures(
            Forward(Memory::new(include_bytes!("fixtures/shuffled.zip"), 7)),
            &manifest,
            &trips,
            &["S1"],
            &mut work,
            |_| {
                events += 1;
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(events, 1);
        // Trips occurs after stop_times: callbacks are provisional until success.
        let mut provisional = 0;
        let error = stream::departures(
            Forward(Memory::new(
                include_bytes!("fixtures/changed-trips.zip"),
                13,
            )),
            &manifest,
            &trips,
            &["S1"],
            &mut work,
            |_| {
                provisional += 1;
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error, Error::SnapshotMismatch(Table::Trips));
        assert_eq!(provisional, 1);
    });
}

#[test]
fn original_streams_validate_footer_body_eof_and_metadata_capacity() {
    block_on(async {
        let mut work = zip::Workspace::new();
        for end in [FIXTURE.len() - 1, FIXTURE.len() - 22, FIXTURE.len() - 50] {
            assert!(
                stream::scan(
                    Forward(Memory::new(&FIXTURE[..end], 11)),
                    &[Table::Routes],
                    None,
                    &mut work,
                    |_| Ok(())
                )
                .await
                .is_err()
            );
        }
        let mut extra = FIXTURE.to_vec();
        extra.push(0);
        assert_eq!(
            stream::scan(
                Forward(Memory::new(&extra, 11)),
                &[],
                None,
                &mut work,
                |_| Ok(())
            )
            .await
            .unwrap_err(),
            Error::InvalidZip
        );
        let mut slots = [TripSlot::<32>::EMPTY; 2];
        let mut trips = TripIndex::new(&mut slots);
        let mut service_slots = [stream::ServiceSlot::<32>::EMPTY; 8];
        let mut services = stream::Services::new(&mut service_slots);
        assert_eq!(
            stream::prepare(
                Forward(Memory::new(FIXTURE, 17)),
                stream::Selection {
                    routes: &["4"],
                    direction: None,
                    date: Some("20261017".parse().unwrap())
                },
                &mut trips,
                &mut services,
                &mut work
            )
            .await
            .unwrap_err(),
            Error::IndexFull
        );
    });
}

#[test]
fn original_metadata_and_queries_skip_shape_inflation() {
    block_on(async {
        let mut corrupt_shape = FIXTURE.to_vec();
        let name = corrupt_shape
            .windows(10)
            .position(|w| w == b"shapes.txt")
            .unwrap();
        corrupt_shape[name + 10..name + 15].fill(0xff);
        let mut work = zip::Workspace::new();
        let mut count = 0;
        stream::scan(
            Forward(Memory::new(&corrupt_shape, 2)),
            &[Table::Routes, Table::Stops],
            None,
            &mut work,
            |_| {
                count += 1;
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(count, 6); // 2 routes + 4 stops, corrupt unneeded shapes never decoded.
    });
}

#[test]
fn bounded_event_cache_clears_after_late_errors_and_overflow() {
    block_on(async {
        let feed = Feed::open(Memory::new(FIXTURE, 64), &mut [0; 128])
            .await
            .unwrap();
        let manifest = zip::Manifest::from_directory(feed.directory());
        let mut slots = [TripSlot::<32>::EMPTY; 2];
        let mut trips = TripIndex::new(&mut slots);
        trips.insert("T_A", 0, Some(0)).unwrap();
        trips.insert("T_C", 0, Some(0)).unwrap();
        trips.finish().unwrap();
        let mut events = [gtfs::query::DepartureSlot::<32>::EMPTY; 1];
        let mut output = gtfs::query::DepartureBuffer::new(&mut events);
        let mut work = zip::Workspace::new();
        stream::collect(
            Forward(Memory::new(FIXTURE, 7)),
            &manifest,
            &trips,
            &["S2"],
            &mut work,
            &mut output,
        )
        .await
        .unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(
            stream::collect(
                Forward(Memory::new(FIXTURE, 7)),
                &manifest,
                &trips,
                &["S1"],
                &mut work,
                &mut output
            )
            .await,
            Err(Error::IndexFull)
        );
        assert!(output.is_empty());
        assert_eq!(
            stream::collect(
                Forward(Memory::new(include_bytes!("fixtures/changed-trips.zip"), 7)),
                &manifest,
                &trips,
                &["S2"],
                &mut work,
                &mut output
            )
            .await,
            Err(Error::SnapshotMismatch(Table::Trips))
        );
        assert!(output.is_empty());
    });
}

#[test]
fn trip_cursor_handles_misses_order_changes_and_long_unselected_ids() {
    let mut slots = [TripSlot::<3>::EMPTY; 2];
    let mut trips = TripIndex::new(&mut slots);
    trips.insert("a", 0, None).unwrap();
    trips.insert("z", 1, Some(1)).unwrap();
    trips.finish().unwrap();
    let mut cursor = trips.cursor().unwrap();
    for id in [
        "a",
        "a",
        "miss",
        "miss",
        "z",
        "unselected ID larger than the selected ID bound",
        "a",
    ] {
        assert_eq!(
            cursor.find(id).unwrap().map(|s| s.route_index),
            trips.find(id).unwrap().map(|s| s.route_index)
        );
    }
}

#[test]
fn ram_budgets_reject_overflow_and_oversized_tasks() {
    let task = async {};
    let usage =
        gtfs::memory::Usage::check(&task, 100, gtfs::memory::Reserves::EMBEDDED, 400_000).unwrap();
    assert_eq!(usage.total, usage.task_state + 100 + 139_264);
    assert!(matches!(
        gtfs::memory::Usage::check(&task, usize::MAX, gtfs::memory::Reserves::EMBEDDED, 400_000),
        Err(Error::RamBudgetExceeded)
    ));
    assert!(matches!(
        gtfs::memory::Usage::check(&task, 400_000, gtfs::memory::Reserves::EMBEDDED, 400_000),
        Err(Error::RamBudgetExceeded)
    ));
}

struct PausedFooter<'a> {
    data: Memory<'a>,
    pause: usize,
}
impl ErrorType for PausedFooter<'_> {
    type Error = Error;
}
impl Read for PausedFooter<'_> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        if self.data.pos as usize >= self.pause {
            return core::future::pending().await;
        }
        self.data.read(out).await
    }
}

#[test]
fn cancelling_a_pending_stream_drops_provisional_events() {
    block_on(async {
        let feed = Feed::open(Memory::new(FIXTURE, 64), &mut [0; 128])
            .await
            .unwrap();
        let manifest = zip::Manifest::from_directory(feed.directory());
        let mut slots = [TripSlot::<32>::EMPTY; 1];
        let mut trips = TripIndex::new(&mut slots);
        trips.insert("T_A", 0, None).unwrap();
        trips.finish().unwrap();
        let mut events = [gtfs::query::DepartureSlot::<32>::EMPTY; 8];
        let mut output = gtfs::query::DepartureBuffer::new(&mut events);
        let mut work = zip::Workspace::new();
        {
            let source = PausedFooter {
                data: Memory::new(FIXTURE, 1),
                pause: FIXTURE.len() - 10,
            };
            let mut query = core::pin::pin!(stream::collect(
                source,
                &manifest,
                &trips,
                &["S1"],
                &mut work,
                &mut output
            ));
            let mut context = core::task::Context::from_waker(core::task::Waker::noop());
            assert!(matches!(
                core::future::Future::poll(query.as_mut(), &mut context),
                core::task::Poll::Pending
            ));
        }
        assert!(output.is_empty());
    });
}

#[test]
fn route_stop_and_date_filtered_timetables() {
    block_on(async {
        let mut feed = Feed::open(Memory::new(FIXTURE, 7), &mut [0; 128])
            .await
            .unwrap();
        let mut work = zip::Workspace::new();
        let mut routes = 0;
        feed.routes(&mut work, |r| {
            routes += 1;
            if r.id == "4" {
                assert_eq!(r.long_name, "North, South");
                assert_eq!(r.description, Some("Quoted \"legend\"\nnext line"));
            }
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(routes, 2);
        let mut service_storage = [Id::<16>::EMPTY; 8];
        let mut services = IdSet::new(&mut service_storage);
        feed.services_on("20261017".parse().unwrap(), &mut services, &mut work)
            .await
            .unwrap();
        assert_eq!(
            services
                .as_slice()
                .iter()
                .map(Id::as_str)
                .collect::<Vec<_>>(),
            ["ALL", "EXTRA"]
        );
        let mut trip_storage = [TripSlot::<32>::EMPTY; 8];
        let mut trips = TripIndex::new(&mut trip_storage);
        feed.select_trips(&["4"], None, Some(&services), &mut trips, &mut work)
            .await
            .unwrap();
        assert_eq!(trips.len(), 3);
        let mut events = Vec::new();
        feed.departures(&trips, &["S1"], &mut work, |d| {
            events.push((
                d.event.trip_id.to_owned(),
                d.event.departure.unwrap().seconds(),
                d.direction,
            ));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            events,
            [
                ("T_A".into(), 90600, Some(0)),
                ("T_B".into(), 29400, Some(1)),
                ("T_C".into(), 32400, Some(0))
            ]
        );
        let mut stop_storage = [Id::<16>::EMPTY; 8];
        let mut stops = IdSet::new(&mut stop_storage);
        feed.route_stops(&trips, &mut stops, &mut work)
            .await
            .unwrap();
        assert_eq!(stops.len(), 3);
        let mut names = Vec::new();
        feed.named_stops(&stops, &mut work, |s| {
            names.push(s.name.unwrap().to_owned());
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(names, ["Święty Marcin", "Most Teatralny", "Branch"]);
        feed.select_trips(&["4"], Some(0), Some(&services), &mut trips, &mut work)
            .await
            .unwrap();
        assert_eq!(trips.len(), 2);
        let mut optional = 0;
        feed.departures(&trips, &["S3"], &mut work, |d| {
            assert_eq!(d.event.departure, None);
            optional += 1;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(optional, 1);
        feed.services_on("20270101".parse().unwrap(), &mut services, &mut work)
            .await
            .unwrap();
        assert!(services.is_empty());
    });
}

#[test]
fn one_byte_chunks_bom_quotes_and_reordered_headers() {
    block_on(async {
        let input = "\u{feff}route_type,route_short_name,route_id,route_long_name\r\n0,4,R,"
            .to_owned()
            + "\"Żółć, \"\"quoted\"\"\nname\"";
        let mut reader = csv::Reader::<_>::new(Memory::new(input.as_bytes(), 1), Table::Routes);
        let route = reader.next().await.unwrap().unwrap().route().unwrap();
        assert_eq!(route.id, "R");
        assert_eq!(route.long_name, "Żółć, \"quoted\"\nname");
        assert!(reader.next().await.unwrap().is_none());
    });
}

#[test]
fn streaming_deflate_descriptors_and_dictionary_wrap() {
    block_on(async {
        for (data, chunk) in [
            (FIXTURE, 1),
            (include_bytes!("fixtures/descriptor.zip").as_slice(), 37),
        ] {
            let mut work = zip::Workspace::new();
            let mut archive = zip::Reader::new(Memory::new(data, chunk), &mut work);
            let mut entries = 0;
            let mut buf = [0; 53];
            let mut shapes = 0;
            while let Some(entry) = archive.next_entry().await.unwrap() {
                entries += 1;
                let mut count = 0;
                loop {
                    let n = archive.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    count += n;
                }
                if entry.name() == b"shapes.txt" {
                    shapes = count;
                }
            }
            assert_eq!(entries, 7);
            assert_eq!(shapes, 70000);
        }
    });
}

#[test]
fn indexed_stored_data_descriptors_and_dates_only() {
    block_on(async {
        for data in [
            include_bytes!("fixtures/stored-descriptor.zip").as_slice(),
            include_bytes!("fixtures/calendar-dates-only.zip").as_slice(),
        ] {
            let mut feed = Feed::open(Memory::new(data, 31), &mut [0; 64])
                .await
                .unwrap();
            let mut work = zip::Workspace::new();
            let mut storage = [Id::<32>::EMPTY; 8];
            let mut services = IdSet::new(&mut storage);
            feed.services_on("20261017".parse().unwrap(), &mut services, &mut work)
                .await
                .unwrap();
            assert!(services.contains("EXTRA"));
            assert!(!services.contains("REMOVED"));
            let mut count = 0;
            feed.routes(&mut work, |_| {
                count += 1;
                Ok(())
            })
            .await
            .unwrap();
            assert_eq!(count, 2);
        }
    });
}

#[test]
fn limits_and_invalid_headers_are_explicit() {
    block_on(async {
        let mut tiny = csv::Reader::<_, 8, 8>::new(
            Memory::new(b"route_id,route_type\n4,0\n", 2),
            Table::Routes,
        );
        assert!(matches!(tiny.next().await, Err(Error::RecordTooLong)));
        let mut fields = csv::Reader::<_, 128, 1>::new(
            Memory::new(b"route_id,route_type\n4,0\n", 3),
            Table::Routes,
        );
        assert!(matches!(fields.next().await, Err(Error::TooManyFields)));
        let mut missing = csv::Reader::<_>::new(Memory::new(b"route_type\n", 1), Table::Routes);
        assert!(matches!(
            missing.next().await,
            Err(Error::MissingColumn("route_id"))
        ));
        let mut utf8 = csv::Reader::<_>::new(
            Memory::new(b"route_id,route_type\na\xff,0\n", 1),
            Table::Routes,
        );
        assert!(matches!(utf8.next().await, Err(Error::InvalidUtf8)));
        let mut width = csv::Reader::<_>::new(
            Memory::new(b"route_id,route_type\na,0,extra\n", 3),
            Table::Routes,
        );
        assert!(matches!(
            width.next().await,
            Err(Error::InvalidField("CSV record width"))
        ));
    });
}

#[test]
fn records_exactly_at_the_byte_and_field_limits_fit() {
    block_on(async {
        for ending in ["\r\n", ""] {
            let input = format!("route_id,route_type\r\n\"abcdefghijklmnopq\",0{ending}");
            let mut reader =
                csv::Reader::<_, 18, 2>::new(Memory::new(input.as_bytes(), 1), Table::Routes);
            assert_eq!(
                reader.next().await.unwrap().unwrap().route().unwrap().id,
                "abcdefghijklmnopq"
            );
            assert!(reader.next().await.unwrap().is_none());
        }
        let mut reader = csv::Reader::<_, 18, 2>::new(
            Memory::new(b"route_id,route_type\nabcdefghijklmnopqr,0\n", 1),
            Table::Routes,
        );
        assert!(matches!(reader.next().await, Err(Error::RecordTooLong)));
    });
}

#[test]
fn corruption_and_truncation_do_not_succeed() {
    block_on(async {
        let mut data = include_bytes!("fixtures/stored.zip").to_vec();
        data[30 + "routes.txt".len() + 12] ^= 1;
        let mut work = zip::Workspace::new();
        let mut archive = zip::Reader::new(Memory::new(&data, 13), &mut work);
        archive.next_entry().await.unwrap();
        let mut buf = [0; 512];
        let error = loop {
            match archive.read(&mut buf).await {
                Ok(0) => panic!("corrupt CRC accepted"),
                Ok(_) => {}
                Err(e) => break e,
            }
        };
        assert_eq!(error, Error::CrcMismatch);
        for n in [0, 10, FIXTURE.len() - 1] {
            assert!(
                Feed::open(Memory::new(&FIXTURE[..n], 31), &mut [0; 64])
                    .await
                    .is_err()
            );
        }
        let data = include_bytes!("fixtures/descriptor.zip");
        let mut archive = zip::Reader::new(Memory::new(&data[..60], 7), &mut work);
        assert!(archive.next_entry().await.is_ok());
        loop {
            match archive.read(&mut buf).await {
                Ok(0) => panic!("truncated entry accepted"),
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

#[test]
fn archive_comments_and_small_directory_scratch() {
    block_on(async {
        let data = include_bytes!("fixtures/comment.zip");
        let mut source = Memory::new(data, 17);
        let dir = zip::Directory::read(&mut source, &mut [0; 43])
            .await
            .unwrap();
        assert!(dir.member(Table::Routes).is_some());
    });
}

#[test]
fn exact_ids_capacity_and_sorting() {
    let mut slots = [TripSlot::<4>::EMPTY; 2];
    let mut trips = TripIndex::new(&mut slots);
    trips.insert("b", 0, None).unwrap();
    assert_eq!(trips.find("b").unwrap_err(), Error::IndexNotSorted);
    assert_eq!(trips.insert("large", 0, None), Err(Error::IdTooLong));
    trips.insert("a", 1, Some(1)).unwrap();
    assert_eq!(trips.insert("c", 0, None), Err(Error::IndexFull));
    trips.finish().unwrap();
    assert_eq!(trips.find("a").unwrap().unwrap().route_index, 1);
    assert!(trips.find("aa").unwrap().is_none());
    trips.clear();
    trips.insert("a", 0, None).unwrap();
    trips.insert("a", 0, None).unwrap();
    assert_eq!(trips.finish(), Err(Error::DuplicateTrip));
    let mut ids = [Id::<4>::EMPTY; 1];
    let mut set = IdSet::new(&mut ids);
    set.insert("a").unwrap();
    set.insert("a").unwrap();
    assert_eq!(set.insert("b"), Err(Error::IndexFull));
    set.remove("a");
    assert!(set.is_empty());
}

#[test]
fn service_dates_and_after_midnight_times() {
    assert_eq!("20261017".parse::<Date>().unwrap().weekday(), 5);
    assert_eq!("20000101".parse::<Date>().unwrap().weekday(), 5);
    assert!("20000229".parse::<Date>().is_ok());
    assert!("19000229".parse::<Date>().is_err());
    for date in ["00000101", "20260230", "20261301", "2026101", "2026-10-17"] {
        assert!(date.parse::<Date>().is_err());
    }
    assert_eq!("25:10:00".parse::<Time>().unwrap().seconds(), 90600);
    assert_eq!(
        "100:00:00".parse::<Time>().unwrap().to_string(),
        "100:00:00"
    );
    for time in [
        "-1:00:00",
        "01:60:00",
        "01:00:60",
        "999999999999:00:00",
        "12:3:00",
    ] {
        assert!(time.parse::<Time>().is_err());
    }
}

#[test]
fn archive_discovery_is_streaming_and_dates_are_validated() {
    block_on(async {
        let input=b"label 20261015_20261031.zip href=20261015_20261031.zip ignore 20260230_20260301.zip next 20261019_20261031.zip";
        let mut listing = poznan::Archives::new(Memory::new(input, 1));
        let name = listing.next().await.unwrap().unwrap();
        assert_eq!(name.as_str(), "20261015_20261031.zip");
        assert!(name.covers("20261017".parse().unwrap()));
        assert_eq!(
            name.url(&mut [0; 192]).unwrap(),
            format!("{}?file=20261015_20261031.zip", poznan::LATEST_URL)
        );
        assert_eq!(
            listing.next().await.unwrap().unwrap().as_str(),
            "20261019_20261031.zip"
        );
        assert!(listing.next().await.unwrap().is_none());
    });
}

struct Remote<'a>(&'a [u8]);
impl http::RangeFetch for Remote<'_> {
    async fn fetch(&mut self, r: http::ByteRange, out: &mut [u8]) -> Result<(), Error> {
        assert_eq!(r.len() as usize, out.len());
        out.copy_from_slice(&self.0[r.start() as usize..=r.end() as usize]);
        Ok(())
    }
}

#[test]
fn selective_http_ranges_decode_only_the_requested_table() {
    block_on(async {
        let mut cache = [0; 256];
        let mut source =
            http::RangeReader::new(Remote(FIXTURE), FIXTURE.len() as u64, &mut cache).unwrap();
        let dir = zip::Directory::read(&mut source, &mut [0; 128])
            .await
            .unwrap();
        let member = dir.member(Table::Routes).unwrap();
        let r = member.header_range();
        let mut header = [0; 30];
        header.copy_from_slice(&FIXTURE[r.start() as usize..=r.end() as usize]);
        let r = member.data_range(&header).unwrap();
        let bytes = &FIXTURE[r.start() as usize..=r.end() as usize];
        let mut work = zip::Workspace::new();
        let zip = zip::Reader::data(Memory::new(bytes, 1), &mut work, member);
        let mut csv = csv::Reader::<_>::new(zip, Table::Routes);
        let mut routes = Vec::new();
        while let Some(row) = csv.next().await.unwrap() {
            routes.push(row.route().unwrap().id.to_owned());
        }
        assert_eq!(routes, ["4", "5"]);
    });
}

#[test]
fn range_headers_reject_mismatches() {
    let range = http::ByteRange::new(12, 34).unwrap();
    assert_eq!(range.header(&mut [0; 64]).unwrap(), "bytes=12-34");
    assert_eq!(range.validate_content_range(b"bytes 12-34/100"), Ok(100));
    for value in [
        b"bytes 0-34/100".as_slice(),
        b"bytes 12-34/34",
        b"bytes 12-34/*",
        b"bytes 12-+34/100",
        b"bytes 12-34/100junk",
    ] {
        assert_eq!(
            range.validate_content_range(value),
            Err(Error::InvalidRange)
        );
    }
    assert_eq!(http::ByteRange::new(4, 3), Err(Error::InvalidRange));
}

#[test]
fn mirror_snapshot_discovery_handles_split_links_and_duplicates() {
    block_on(async {
        let a = "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610170128/mdb-982-202610170128.zip";
        let b = "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610150044/mdb-982-202610150044.zip";
        let input = format!(
            "<a href=\"{a}\">Latest</a><a href=\"{a}\">Download</a><a href=\"{b}\">Older</a>"
        );
        let mut listing = poznan::MirrorSnapshots::new(Memory::new(input.as_bytes(), 1));
        assert_eq!(listing.next().await.unwrap().unwrap().url(), a);
        assert_eq!(listing.next().await.unwrap().unwrap().url(), b);
        assert!(listing.next().await.unwrap().is_none());
        assert!(
            a.replace("202610170128.zip", "202610150044.zip")
                .parse::<poznan::MirrorSnapshot>()
                .is_err()
        );
    });
}
