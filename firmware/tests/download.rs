use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use embedded_io_async::{ErrorType, Read};
use gtfs::{Table, http::ByteRange, zip};
use std::cell::{Cell, RefCell};
use timetable_display::{Config, catalog::Catalog, config, feed::Builder};
use timetable_firmware::download::{self, Error, Http, Metadata, Operation};

const ORIGINAL: &[u8] = include_bytes!("../../display/tests/fixtures/edge.zip");
const MIRROR: &[u8] = include_bytes!("../../display/tests/fixtures/mirror.zip");
const TOMORROW: &[u8] = include_bytes!("../../display/tests/fixtures/mirror-tomorrow.zip");
const OLD: &str =
    "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610150044/mdb-982-202610150044.zip";
const NEXT: &str =
    "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610170128/mdb-982-202610170128.zip";
const NOW: i64 = 1792269000; // 2026-10-17 20:30 UTC, next service day is required.
struct Body<'a>(&'a [u8]);
impl ErrorType for Body<'_> {
    type Error = embedded_io_async::ErrorKind;
}
impl Read for Body<'_> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let n = out.len().min(self.0.len()).min(7);
        out[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        Ok(n)
    }
}
fn run<T>(future: impl Future<Output = T>) -> T {
    let mut f = pin!(future);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            return v;
        }
    }
}
#[derive(Default)]
struct Client {
    calls: RefCell<Vec<(String, Option<ByteRange>)>>,
    disable: bool,
    split: bool,
    corrupt: bool,
    fail_original: bool,
    late_error: bool,
    cancel: bool,
    fallback: Cell<bool>,
}
impl Http for Client {
    fn fallback(&self, _: &Error) {
        self.fallback.set(true);
    }
    async fn fetch(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        op: &mut impl Operation,
    ) -> Result<Metadata, Error> {
        self.calls
            .borrow_mut()
            .push((url.into(), range.map(|(r, _)| r)));
        if url == gtfs::poznan::MIRROR_PAGE {
            if self.disable {
                return Err(Error::Network);
            }
            let text = if self.split {
                format!("{OLD} {NEXT} {OLD}")
            } else {
                OLD.into()
            };
            op.read(Body(text.as_bytes())).await?;
            return Ok(Metadata::default());
        }
        if url == gtfs::poznan::LIST_URL {
            op.read(Body(b"20261015_20261031.zip")).await?;
            return Ok(Metadata::default());
        }
        if url.starts_with(gtfs::poznan::LATEST_URL) {
            if self.fail_original {
                return Err(Error::Network);
            }
            op.read(Body(ORIGINAL)).await?;
            return Ok(Metadata::default());
        }
        let data = if url == NEXT {
            TOMORROW
        } else {
            assert_eq!(url, OLD);
            MIRROR
        };
        let (range, expected) = range.expect("mirror never downloads an entire ZIP");
        assert!(expected.is_none_or(|n| n == data.len() as u64));
        let bytes = &data[range.start() as usize..=range.end() as usize];
        let mut altered = bytes.to_vec();
        // Fail the second snapshot's large stop_times payload, after tomorrow's
        // results have already been staged in Builder.
        if self.corrupt && url == OLD && bytes.get(30..44) == Some(b"stop_times.txt") {
            let last = altered.len() - 1;
            altered[last] ^= 1;
        }
        op.read(Body(&altered)).await?;
        if url == OLD && bytes.get(30..44) == Some(b"stop_times.txt") {
            if self.cancel {
                core::future::pending::<()>().await;
            }
            if self.late_error {
                return Err(Error::Network);
            }
        }
        Ok(Metadata {
            length: Some(data.len() as u64),
            modified: Some(NOW),
        })
    }
}
fn settings() -> Config {
    let mut bytes = *include_bytes!("../../display/tests/settings.bin");
    let mut c = config::decode(&mut bytes).unwrap().0;
    c.groups.truncate(1);
    c.groups[0].picks.truncate(1);
    let p = &mut c.groups[0].picks[0];
    p.route = "R4".try_into().unwrap();
    p.line = "4".try_into().unwrap();
    p.stop = "S1".try_into().unwrap();
    p.stop_code.clear();
    p.direction = 0;
    p.stop_name = Catalog::new(include_bytes!("../../display/tests/fixtures/catalog.bin"))
        .unwrap()
        .stop(0)
        .unwrap()
        .name
        .try_into()
        .unwrap();
    c
}
fn collect(
    client: &Client,
) -> (
    Builder,
    Result<timetable_display::freshness::Freshness, Error>,
) {
    let mut builder = Builder::new("20261017".parse().unwrap());
    let mut workspace = zip::Workspace::new();
    let result = run(download::refresh(
        client,
        &mut builder,
        &mut workspace,
        &settings(),
        NOW,
    ));
    builder.events.sort();
    (builder, result)
}
#[test]
fn mirror_fetches_each_table_once_and_matches_the_official_join() {
    let client = Client::default();
    let (mirror, result) = collect(&client);
    let fresh = result.unwrap();
    assert!(!client.fallback.get());
    assert_eq!(
        fresh.source_updated_at, None,
        "mirror fetch time is not source modification time"
    );
    let (official, result) = collect(&Client {
        disable: true,
        ..Client::default()
    });
    result.unwrap();
    assert_eq!(
        mirror.events.iter().collect::<Vec<_>>(),
        official.events.iter().collect::<Vec<_>>()
    );
    assert!(!mirror.events.is_empty());
    let mut directory_source = Slice(MIRROR, 0);
    let directory = run(zip::Directory::read(&mut directory_source, &mut [0; 128])).unwrap();
    for table in Table::ALL {
        let member = directory.member(table).unwrap();
        let payload = member.record_range();
        assert_eq!(
            client
                .calls
                .borrow()
                .iter()
                .filter(|(url, r)| url == OLD && *r == Some(payload))
                .count(),
            1,
            "{table:?} payload fetched repeatedly"
        );
    }
}
#[test]
fn future_only_latest_snapshot_uses_history_and_late_corruption_restarts_cleanly() {
    let good = Client {
        split: true,
        ..Client::default()
    };
    let (expected, result) = collect(&good);
    result.unwrap();
    assert!(!good.fallback.get());
    let bad = Client {
        split: true,
        corrupt: true,
        ..Client::default()
    };
    let (fallback, result) = collect(&bad);
    result.unwrap();
    assert!(bad.fallback.get());
    assert_eq!(
        expected.events.iter().collect::<Vec<_>>(),
        fallback.events.iter().collect::<Vec<_>>()
    );
    assert_eq!(
        fallback.events.len(),
        2,
        "provisional tomorrow events must not be duplicated by fallback"
    );
    let (failed, result) = collect(&Client {
        disable: true,
        fail_original: true,
        ..Client::default()
    });
    assert!(result.is_err());
    assert!(failed.events.is_empty());
}
struct Slice(&'static [u8], usize);
impl ErrorType for Slice {
    type Error = embedded_io_async::ErrorKind;
}
impl Read for Slice {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let n = out.len().min(self.0.len().saturating_sub(self.1));
        out[..n].copy_from_slice(&self.0[self.1..self.1 + n]);
        self.1 += n;
        Ok(n)
    }
}
impl embedded_io_async::Seek for Slice {
    async fn seek(&mut self, from: embedded_io_async::SeekFrom) -> Result<u64, Self::Error> {
        self.1 = match from {
            embedded_io_async::SeekFrom::Start(n) => n as usize,
            embedded_io_async::SeekFrom::End(n) => (self.0.len() as i64 + n) as usize,
            embedded_io_async::SeekFrom::Current(n) => (self.1 as i64 + n) as usize,
        };
        Ok(self.1 as u64)
    }
}

#[test]
fn error_after_payload_callbacks_and_cancelled_final_range_never_commit_provisional_results() {
    let (expected, result) = collect(&Client::default());
    result.unwrap();
    let late = Client {
        split: true,
        late_error: true,
        ..Client::default()
    };
    let (actual, result) = collect(&late);
    result.unwrap();
    assert!(late.fallback.get());
    assert_eq!(
        actual.events.iter().collect::<Vec<_>>(),
        expected.events.iter().collect::<Vec<_>>()
    );
    let cancel = Client {
        split: true,
        cancel: true,
        ..Client::default()
    };
    let c = settings();
    let mut builder = Builder::new("20261017".parse().unwrap());
    let mut workspace = zip::Workspace::new();
    {
        let mut future = pin!(download::refresh(
            &cancel,
            &mut builder,
            &mut workspace,
            &c,
            NOW
        ));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    let official = Client {
        disable: true,
        ..Client::default()
    };
    run(download::refresh(
        &official,
        &mut builder,
        &mut workspace,
        &c,
        NOW,
    ))
    .unwrap();
    builder.events.sort();
    assert_eq!(
        builder.events.iter().collect::<Vec<_>>(),
        expected.events.iter().collect::<Vec<_>>()
    );
    assert_eq!(builder.headsigns.get(1), expected.headsigns.get(1));
}

#[test]
fn empty_selection_never_reports_a_timetable_check_or_contacts_http() {
    let client = Client::default();
    let mut c = settings();
    c.groups[0].picks.clear();
    let mut builder = Builder::new("20261017".parse().unwrap());
    let mut workspace = zip::Workspace::new();
    let result = run(download::refresh(
        &client,
        &mut builder,
        &mut workspace,
        &c,
        NOW,
    ))
    .unwrap();
    assert_eq!(result.checked_at, None);
    assert_eq!(result.source_updated_at, None);
    assert!(client.calls.borrow().is_empty());
}

#[test]
fn mirror_decoded_work_yields_even_with_an_always_ready_http_source() {
    use timetable_display::feed::Tables;
    use timetable_firmware::cooperative::Cooperative;
    const ZIP: &[u8] = include_bytes!("../../checks/device-cpu/stop_times-repeated.zip");
    struct ReadyClient;
    impl Http for ReadyClient {
        async fn fetch(
            &self,
            _: &str,
            range: Option<(ByteRange, Option<u64>)>,
            op: &mut impl Operation,
        ) -> Result<Metadata, Error> {
            let (r, _) = range.unwrap();
            op.read(Cooperative::new(
                &ZIP[r.start() as usize..=r.end() as usize],
            ))
            .await?;
            Ok(Metadata {
                length: Some(ZIP.len() as u64),
                modified: None,
            })
        }
    }
    let mut workspace = zip::Workspace::new();
    let mut mirror = run(download::Mirror::open(
        &ReadyClient,
        OLD.parse().unwrap(),
        &mut workspace,
    ))
    .unwrap();
    let rows = Cell::new(0);
    let sum = Cell::new(0);
    let mut longest = 0;
    let mut yields = 0;
    {
        let mut future = pin!(
            mirror.scan(&[Table::StopTimes], None, &mut workspace, |row| {
                rows.set(rows.get() + 1);
                sum.set(sum.get() + row.required(gtfs::csv::Column::TripId)?.len());
                Ok(())
            })
        );
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            let before = rows.get();
            let result = future.as_mut().poll(&mut cx);
            longest = longest.max(rows.get() - before);
            if let Poll::Ready(result) = result {
                result.unwrap();
                break;
            }
            yields += 1;
        }
    }
    assert_eq!((rows.get(), sum.get()), (8192, 98080));
    assert!(
        yields > 32,
        "decoded rows must yield independently of HTTP reads"
    );
    assert!(
        longest <= 32,
        "too many CSV callbacks without returning to the runner: {longest}"
    );
}
