use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use embedded_io_async::{ErrorType, Read};
use gtfs::{Error, http::ByteRange, zip};
use std::cell::{Cell, RefCell};
use timetable_display::{Config, catalog::Catalog, config, feed::Builder};
use timetable_firmware::{
    cache::{self, Storage},
    download::{self, Http, Metadata, Operation},
};
const ORIGINAL: &[u8] = include_bytes!("../../display/tests/fixtures/cache-full.zip");
const CHANGED: &[u8] = include_bytes!("../../display/tests/fixtures/cache-changed.zip");
const UNRELATED: &[u8] = include_bytes!("../../display/tests/fixtures/cache-unrelated.zip");
const URL: &str =
    "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610150044/mdb-982-202610150044.zip";
const NOW: i64 = 1792269000;
fn run<T>(future: impl Future<Output = T>) -> T {
    let mut f = pin!(future);
    loop {
        if let Poll::Ready(value) = f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            return value;
        }
    }
}
struct Memory {
    identity: Cell<&'static [u8]>,
    bytes: RefCell<Vec<u8>>,
    erases: Cell<usize>,
    writes: Cell<usize>,
    read_bytes: Cell<usize>,
    read_fault: Cell<Option<u32>>,
    scratch_calls: Cell<usize>,
    scratch_fails: Cell<bool>,
    metadata_fails: Cell<bool>,
    scratch_active: Cell<usize>,
    fault: Cell<Option<usize>>,
    operations: Cell<usize>,
    current: Cell<bool>,
    checked_at: Cell<Option<i64>>,
    pause_commit: Cell<bool>,
    commit_waiting: Cell<bool>,
    approval: Cell<bool>,
    approval_requests: Cell<usize>,
    approval_previous: Cell<Option<i64>>,
}
impl Memory {
    fn new() -> Self {
        Self {
            identity: Cell::new(&[]),
            bytes: RefCell::new(vec![0xff; cache::SIZE as usize]),
            erases: Cell::new(0),
            writes: Cell::new(0),
            read_bytes: Cell::new(0),
            read_fault: Cell::new(None),
            scratch_calls: Cell::new(0),
            scratch_fails: Cell::new(false),
            metadata_fails: Cell::new(false),
            scratch_active: Cell::new(0),
            fault: Cell::new(None),
            operations: Cell::new(0),
            current: Cell::new(true),
            checked_at: Cell::new(None),
            pause_commit: Cell::new(false),
            commit_waiting: Cell::new(false),
            // Existing power-loss/CRC tests explicitly permit repeated writes.
            approval: Cell::new(true),
            approval_requests: Cell::new(0),
            approval_previous: Cell::new(None),
        }
    }
    fn from(bytes: &[u8]) -> Self {
        let s = Self::new();
        s.bytes.borrow_mut().copy_from_slice(bytes);
        s
    }
    fn step(&self) -> bool {
        let n = self.operations.get();
        self.operations.set(n + 1);
        self.fault.get() == Some(n)
    }
    fn counters(&self) -> (usize, usize) {
        (self.erases.get(), self.writes.get())
    }
}
struct Scratch<'a> {
    bytes: Vec<u8>,
    active: &'a Cell<usize>,
}
impl AsMut<[u8]> for Scratch<'_> {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}
impl Drop for Scratch<'_> {
    fn drop(&mut self) {
        self.active.set(self.active.get() - 1);
    }
}
impl Storage for Memory {
    fn metadata(&self) -> Result<impl AsMut<[cache::Metadata]>, Error> {
        if self.metadata_fails.get() {
            return Err(Error::BufferTooSmall);
        }
        Ok([cache::Metadata::default()])
    }
    fn identity(&self) -> &[u8] {
        self.identity.get()
    }
    fn scratch(&self) -> Result<impl AsMut<[u8]>, Error> {
        self.scratch_calls.set(self.scratch_calls.get() + 1);
        if self.scratch_fails.get() {
            return Err(Error::BufferTooSmall);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(cache::FILTER_BYTES)
            .map_err(|_| Error::BufferTooSmall)?;
        bytes.resize(cache::FILTER_BYTES, 0);
        self.scratch_active.set(self.scratch_active.get() + 1);
        Ok(Scratch {
            bytes,
            active: &self.scratch_active,
        })
    }
    fn read(&self, offset: u32, out: &mut [u8]) -> Result<(), Error> {
        if self.read_fault.get() == Some(offset) {
            self.read_fault.set(None);
            return Err(Error::Io(embedded_io_async::ErrorKind::Other));
        }
        let start = (offset - cache::OFFSET) as usize;
        self.read_bytes.set(self.read_bytes.get() + out.len());
        out.copy_from_slice(
            self.bytes
                .borrow()
                .get(start..start + out.len())
                .ok_or(Error::InvalidRange)?,
        );
        Ok(())
    }
    fn write(&self, offset: u32, bytes: &[u8]) -> Result<(), Error> {
        assert_eq!(offset % 4, 0);
        assert_eq!(bytes.len() % 4, 0);
        self.writes.set(self.writes.get() + 1);
        let fail = self.step();
        let n = if fail { bytes.len() / 2 } else { bytes.len() };
        let mut target = self.bytes.borrow_mut();
        let start = (offset - cache::OFFSET) as usize;
        for (d, s) in target[start..start + n].iter_mut().zip(&bytes[..n]) {
            assert_eq!(*d & *s, *s, "programming requires erased bits");
            *d &= *s;
        }
        if fail {
            Err(Error::Io(embedded_io_async::ErrorKind::Other))
        } else {
            Ok(())
        }
    }
    fn erase(&self, start: u32, end: u32) -> Result<(), Error> {
        assert_eq!(start % 4096, 0);
        assert_eq!(end - start, 4096);
        self.erases.set(self.erases.get() + 1);
        let fail = self.step();
        let start = (start - cache::OFFSET) as usize;
        let len = if fail { 2048 } else { 4096 };
        self.bytes.borrow_mut()[start..start + len].fill(0xff);
        if fail {
            Err(Error::Io(embedded_io_async::ErrorKind::Other))
        } else {
            Ok(())
        }
    }
    async fn current(&self) -> bool {
        core::future::poll_fn(|cx| {
            if self.pause_commit.get() {
                self.commit_waiting.set(true);
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(self.current.get())
            }
        })
        .await
    }
    fn checked_at(&self) -> Option<i64> {
        self.checked_at.get()
    }
    fn approve_update(&self, previous: i64) -> bool {
        self.approval_requests.set(self.approval_requests.get() + 1);
        self.approval_previous.set(Some(previous));
        self.approval.get()
    }
}
struct Body<'a>(&'a [u8]);
impl ErrorType for Body<'_> {
    type Error = embedded_io_async::ErrorKind;
}
impl Read for Body<'_> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let n = out.len().min(17).min(self.0.len());
        out[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        Ok(n)
    }
}
struct Client {
    data: &'static [u8],
    conditional: bool,
    drift: u8,
    checks: Cell<usize>,
    bytes: Cell<usize>,
    calls: RefCell<Vec<Option<ByteRange>>>,
    late_error: bool,
    stop_requests: Cell<usize>,
    fail_stop_request: Option<usize>,
    cancel_stop_request: Option<usize>,
    listing: Option<&'static str>,
    alternate: Option<(&'static str, &'static [u8])>,
}
impl Client {
    fn new(data: &'static [u8]) -> Self {
        Self {
            data,
            conditional: false,
            drift: 0,
            checks: Cell::new(0),
            bytes: Cell::new(0),
            calls: RefCell::new(Vec::new()),
            late_error: false,
            stop_requests: Cell::new(0),
            fail_stop_request: None,
            cancel_stop_request: None,
            listing: None,
            alternate: None,
        }
    }
}
impl Http for Client {
    async fn unchanged(
        &self,
        url: &str,
        validator: &download::Validator,
    ) -> Result<bool, download::Error> {
        assert_eq!(url, URL);
        assert_eq!(
            validator.header(),
            Some(("If-None-Match", "\"fixture-v1\""))
        );
        self.checks.set(self.checks.get() + 1);
        Ok(self.conditional)
    }
    async fn fetch(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        op: &mut impl Operation,
    ) -> Result<Metadata, download::Error> {
        self.calls.borrow_mut().push(range.map(|r| r.0));
        if url == gtfs::poznan::MIRROR_PAGE {
            op.read(Body(self.listing.unwrap_or(URL).as_bytes()))
                .await?;
            return Ok(Metadata::default());
        }
        if url == gtfs::poznan::LIST_URL {
            return Err(download::Error::Network);
        }
        let data = if url == URL {
            self.data
        } else {
            let (alternate, data) = self.alternate.expect("known snapshot");
            assert_eq!(url, alternate);
            data
        };
        let (r, size) = range.unwrap();
        assert!(size.is_none_or(|n| n == data.len() as u64));
        let bytes = &data[r.start() as usize..=r.end() as usize];
        self.bytes.set(self.bytes.get() + bytes.len());
        op.read(Body(bytes)).await?;
        if bytes.get(30..44) == Some(b"stop_times.txt") {
            let count = self.stop_requests.get() + 1;
            self.stop_requests.set(count);
            if self.cancel_stop_request == Some(count) {
                core::future::pending::<()>().await;
            }
            if self.late_error || self.fail_stop_request == Some(count) {
                return Err(download::Error::Network);
            }
        }
        Ok(Metadata {
            length: Some(data.len() as u64),
            modified: None,
            validator: download::Validator::new(
                "etag",
                if self.drift == 4 {
                    b"W/\"fixture-v1\""
                } else if self.drift == 1 && r.start() != 0
                    || self.drift == 2 && bytes.get(30..44) == Some(b"stop_times.txt")
                {
                    b"\"fixture-v2\""
                } else {
                    b"\"fixture-v1\""
                },
            ),
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
fn update(
    flash: &Memory,
    client: &Client,
    c: &Config,
    now: i64,
) -> (
    Builder,
    Result<timetable_display::freshness::Freshness, download::Error>,
) {
    let mut builder = Builder::new("20261017".parse().unwrap());
    let mut ws = zip::Workspace::new();
    let result = run(download::refresh(
        &cache::Http {
            inner: client,
            flash,
            enabled: true,
        },
        &mut builder,
        &mut ws,
        c,
        now,
    ));
    assert_eq!(
        flash.scratch_active.get(),
        0,
        "scratch released on completion/error"
    );
    (builder, result)
}
fn restore(flash: &Memory, c: &Config, now: i64) -> Builder {
    let bank = run(cache::load(flash)).unwrap().expect("valid bank");
    let mut b = Builder::new("20261017".parse().unwrap());
    let mut ws = zip::Workspace::new();
    run(cache::restore(flash, bank, &mut b, &mut ws, c, now)).unwrap();
    b.events.sort();
    b
}
fn events(b: &mut Builder) -> Vec<timetable_display::Departure> {
    b.events.sort();
    b.events.iter().collect()
}
#[test]
fn truncated_line_labels_survive_persistent_cache_and_unchanged_refresh() {
    let flash = Memory::new();
    let mut c = settings();
    c.groups[0].picks[0].line = "ABCDE...".try_into().unwrap();
    let client = Client::new(include_bytes!(
        "../../display/tests/fixtures/cache-long-line.zip"
    ));
    let (mut fresh, result) = update(&flash, &client, &c, NOW);
    result.unwrap();
    let expected = events(&mut fresh);
    assert!(!expected.is_empty());
    let counters = flash.counters();
    assert_eq!(events(&mut restore(&flash, &c, NOW)), expected);
    let (mut unchanged, result) = update(&flash, &client, &c, NOW + 60);
    result.unwrap();
    assert_eq!(events(&mut unchanged), expected);
    assert_eq!(flash.counters(), counters);
}
#[test]
fn cold_boot_replays_full_period_and_unchanged_checks_do_not_write() {
    let flash = Memory::new();
    let c = settings();
    let (mut original, result) = update(&flash, &Client::new(ORIGINAL), &c, NOW);
    result.unwrap();
    let counters = flash.counters();
    assert!(counters.0 > 0 && counters.1 > 0);
    let mut cold = restore(&flash, &c, NOW);
    assert_eq!(events(&mut original), events(&mut cold));
    let later = NOW + 12 * 3600;
    let mut cached = restore(&flash, &c, later);
    assert!(
        !cached.events.is_empty(),
        "daytime trip was outside the original six-hour horizon"
    );
    let client = Client::new(ORIGINAL);
    let (mut fresh, result) = update(&flash, &client, &c, later);
    assert_eq!(result.unwrap().checked_at, Some(later));
    assert_eq!(events(&mut cached), events(&mut fresh));
    assert_eq!(
        flash.counters(),
        counters,
        "time and check timestamp must never erase/program flash"
    );
    assert!(
        client.bytes.get() < ORIGINAL.len(),
        "unchanged identity should skip large members"
    );
    let mut changed_settings = c.clone();
    changed_settings.groups[0].name = "Different label".try_into().unwrap();
    changed_settings.groups[0].picks[0].walk = 12;
    changed_settings.groups[0].picks[0].direction = -1;
    let duplicate = changed_settings.groups[0].picks[0].clone();
    changed_settings.groups[0].picks.push(duplicate).unwrap();
    update(&flash, &Client::new(ORIGINAL), &changed_settings, later)
        .1
        .unwrap();
    assert_eq!(
        flash.counters(),
        counters,
        "walking, grouping, direction and duplicate choices do not change stored coverage"
    );
}

#[test]
fn frequent_changed_cache_requires_approval_before_any_flash_write() {
    let initial = Memory::new();
    let config = settings();
    initial.checked_at.set(Some(NOW + 10));
    update(&initial, &Client::new(ORIGINAL), &config, NOW)
        .1
        .unwrap();
    assert_eq!(
        initial.approval_requests.get(),
        0,
        "first fill is automatic"
    );
    let saved = initial.bytes.borrow().clone();
    for age in [-1, 0, 1, 86399, 86400, 86401] {
        let flash = Memory::from(&saved);
        flash.approval.set(false);
        let client = Client::new(CHANGED);
        let (builder, result) = update(&flash, &client, &config, NOW + 10 + age);
        if age < 86400 {
            assert!(
                matches!(result, Err(download::Error::CacheUpdateRequired(previous)) if previous == NOW + 10)
            );
            assert_eq!(flash.approval_requests.get(), 1);
            assert_eq!(flash.approval_previous.get(), Some(NOW + 10));
            assert_eq!(flash.counters(), (0, 0));
            assert_eq!(*flash.bytes.borrow(), saved);
            assert!(
                builder.events.is_empty(),
                "unapproved results stay provisional"
            );
            assert_eq!(
                client.stop_requests.get(),
                1,
                "no write pass or official fallback"
            );
        } else {
            result.unwrap();
            assert_eq!(flash.approval_requests.get(), 0);
            assert!(flash.writes.get() > 0);
        }
    }
    let flash = Memory::from(&saved);
    flash.approval.set(false);
    for data in [ORIGINAL, UNRELATED] {
        update(&flash, &Client::new(data), &config, NOW + 60)
            .1
            .unwrap();
    }
    assert_eq!(
        flash.approval_requests.get(),
        0,
        "unchanged selected data never prompts"
    );
    assert_eq!(flash.counters(), (0, 0));
    flash.approval.set(true);
    update(&flash, &Client::new(CHANGED), &config, NOW + 60)
        .1
        .unwrap();
    assert_eq!(flash.approval_requests.get(), 1);
    assert_eq!(
        run(cache::load(&flash)).unwrap().unwrap().checked_at(),
        NOW + 60
    );
    assert!(flash.writes.get() > 0);
}

#[test]
fn clockless_boot_shows_flash_board_while_background_update_waits_for_approval() {
    use timetable_display::{App, model::Status};
    let flash = Memory::new();
    let config = settings();
    update(&flash, &Client::new(ORIGINAL), &config, NOW)
        .1
        .unwrap();
    let counters = flash.counters();
    let bank = run(cache::load(&flash)).unwrap().unwrap();
    let mut builder = restore(&flash, &config, bank.checked_at());
    let mut app = App::default();
    app.publish(
        core::mem::take(&mut builder.events),
        core::mem::take(&mut builder.headsigns),
        builder.now,
    );
    app.status = Status::Loading;
    let saved: Vec<_> = app.board_departures().collect();
    let freshness = app.freshness;
    assert!(!saved.is_empty());
    assert!(
        app.clock.is_none(),
        "replay does not manufacture current UTC"
    );
    flash.approval.set(false);
    let client = Client::new(CHANGED);
    assert!(matches!(
        update(&flash, &client, &config, NOW + 60).1,
        Err(download::Error::CacheUpdateRequired(_))
    ));
    assert_eq!(app.board_departures().collect::<Vec<_>>(), saved);
    assert_eq!(app.freshness, freshness);
    assert_eq!(flash.counters(), counters);
}
#[test]
fn unrelated_source_rows_are_compared_without_wearing_flash() {
    let flash = Memory::new();
    let c = settings();
    let (mut before, result) = update(&flash, &Client::new(ORIGINAL), &c, NOW);
    result.unwrap();
    let counters = flash.counters();
    let (mut after, result) = update(&flash, &Client::new(UNRELATED), &c, NOW + 1);
    result.unwrap();
    assert_eq!(events(&mut before), events(&mut after));
    assert_eq!(flash.counters(), counters);
}
#[test]
fn changed_selected_content_commits_once_and_corrupt_newer_falls_back() {
    let flash = Memory::new();
    let c = settings();
    update(&flash, &Client::new(ORIGINAL), &c, NOW).1.unwrap();
    let original = flash.bytes.borrow().clone();
    let mut old = restore(&flash, &c, NOW);
    update(&flash, &Client::new(CHANGED), &c, NOW + 1)
        .1
        .unwrap();
    let new_counters = flash.counters();
    let new = restore(&flash, &c, NOW + 1);
    assert!(
        new.events
            .iter()
            .any(|e| new.headsigns.get(e.headsign).unwrap().contains("Changed"))
    );
    update(&flash, &Client::new(CHANGED), &c, NOW + 2)
        .1
        .unwrap();
    assert_eq!(flash.counters(), new_counters);
    let newer = cache::BANK_SIZE as usize;
    flash.bytes.borrow_mut()[newer + 4096 + 5] ^= 1;
    let mut recovered = restore(&flash, &c, NOW);
    assert_eq!(events(&mut recovered), events(&mut old));
    assert_eq!(
        &flash.bytes.borrow()[..cache::BANK_SIZE as usize],
        &original[..cache::BANK_SIZE as usize]
    );
}
#[test]
fn every_interrupted_erase_or_program_preserves_the_previous_bank() {
    let base = Memory::new();
    let c = settings();
    update(&base, &Client::new(ORIGINAL), &c, NOW).1.unwrap();
    let saved = base.bytes.borrow().clone();
    let trial = Memory::from(&saved);
    update(&trial, &Client::new(CHANGED), &c, NOW + 1)
        .1
        .unwrap();
    let steps = trial.operations.get();
    for fail in 0..steps {
        let flash = Memory::from(&saved);
        flash.fault.set(Some(fail));
        let _ = update(&flash, &Client::new(CHANGED), &c, NOW + 1);
        let b = restore(&flash, &c, NOW);
        assert!(
            !b.headsigns.get(0).unwrap().contains("Changed"),
            "interrupted step {fail}/{steps} published new data"
        );
        assert_eq!(
            &flash.bytes.borrow()[..cache::BANK_SIZE as usize],
            &saved[..cache::BANK_SIZE as usize]
        );
    }
}
#[test]
fn late_http_error_and_obsolete_selection_never_commit() {
    let flash = Memory::new();
    let c = settings();
    update(&flash, &Client::new(ORIGINAL), &c, NOW).1.unwrap();
    let saved = flash.bytes.borrow().clone();
    let mut client = Client::new(CHANGED);
    client.late_error = true;
    assert!(update(&flash, &client, &c, NOW + 1).1.is_err());
    assert_eq!(
        *flash.bytes.borrow(),
        saved,
        "comparison failed before any erase"
    );
    flash.current.set(false);
    assert!(
        update(&flash, &Client::new(CHANGED), &c, NOW + 1)
            .1
            .is_err()
    );
    assert!(run(cache::load(&flash)).unwrap().is_some());
    assert!(
        !restore(&flash, &c, NOW)
            .headsigns
            .get(0)
            .unwrap()
            .contains("Changed")
    );
}

fn online(client: &Client, c: &Config, now: i64) -> Builder {
    let mut builder = Builder::new("20261017".parse().unwrap());
    run(download::refresh(
        client,
        &mut builder,
        &mut zip::Workspace::new(),
        c,
        now,
    ))
    .unwrap();
    builder
}
#[test]
fn duplicate_codes_preserve_exact_platform_identity() {
    let data = include_bytes!("../../display/tests/fixtures/cache-code.zip");
    let flash = Memory::new();
    let mut c = settings();
    let p = &mut c.groups[0].picks[0];
    p.route = "R5".try_into().unwrap();
    p.stop = "S2".try_into().unwrap();
    p.stop_code = "T02".try_into().unwrap();
    p.direction = -1;
    update(&flash, &Client::new(data), &c, NOW).1.unwrap();
    c.groups[0].picks[0].stop = "S3".try_into().unwrap();
    let (mut cached, result) = update(&flash, &Client::new(data), &c, NOW);
    result.unwrap();
    let mut direct = online(&Client::new(data), &c, NOW);
    assert!(!cached.events.is_empty());
    assert_eq!(events(&mut cached), events(&mut direct));
}
#[test]
fn republished_older_content_takes_current_snapshot_priority() {
    const A: &str =
        "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610170128/mdb-982-202610170128.zip";
    const C: &str =
        "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610180128/mdb-982-202610180128.zip";
    const BDATA: &[u8] = include_bytes!("../../display/tests/fixtures/cache-priority-old.zip");
    const ADATA: &[u8] = include_bytes!("../../display/tests/fixtures/cache-priority-new.zip");
    const INITIAL: &str = "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610170128/mdb-982-202610170128.zip https://files.mobilitydatabase.org/mdb-982/mdb-982-202610150044/mdb-982-202610150044.zip";
    const RECENT: &str = "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610180128/mdb-982-202610180128.zip https://files.mobilitydatabase.org/mdb-982/mdb-982-202610150044/mdb-982-202610150044.zip";
    let flash = Memory::new();
    let c = settings();
    let mut initial = Client::new(BDATA);
    initial.listing = Some(INITIAL);
    initial.alternate = Some((A, ADATA));
    let (mut old, result) = update(&flash, &initial, &c, NOW);
    result.unwrap();
    let mut recent = Client::new(BDATA);
    recent.listing = Some(RECENT);
    recent.alternate = Some((C, BDATA));
    let (mut cached, result) = update(&flash, &recent, &c, NOW);
    result.unwrap();
    let mut direct = online(&recent, &c, NOW);
    assert_ne!(events(&mut old), events(&mut direct));
    assert_eq!(events(&mut cached), events(&mut direct));
}
#[test]
fn canonical_csv_preserves_quoted_and_multiline_fields() {
    let data = include_bytes!("../../display/tests/fixtures/cache-quoted.zip");
    let flash = Memory::new();
    let c = settings();
    let (mut cached, result) = update(&flash, &Client::new(data), &c, NOW);
    result.unwrap();
    let mut direct = online(&Client::new(data), &c, NOW);
    assert_eq!(events(&mut cached), events(&mut direct));
    for event in cached.events.iter() {
        assert_eq!(
            cached.headsigns.get(event.headsign),
            direct.headsigns.get(event.headsign)
        );
    }
}
#[test]
fn all_period_filter_is_not_limited_to_1024_total_trips() {
    let data = include_bytes!("../../display/tests/fixtures/cache-many.zip");
    let flash = Memory::new();
    let c = settings();
    let (mut cached, result) = update(&flash, &Client::new(data), &c, NOW);
    result.unwrap();
    let mut direct = online(&Client::new(data), &c, NOW);
    assert_eq!(events(&mut cached), events(&mut direct));
    let bank = run(cache::load(&flash)).unwrap().unwrap();
    let mut builder = Builder::new("20261017".parse().unwrap());
    let mut ws = zip::Workspace::new();
    assert_eq!(
        run(cache::restore(
            &flash,
            bank,
            &mut builder,
            &mut ws,
            &c,
            NOW + 3 * 86400
        )),
        Err(Error::IndexFull)
    );
}
#[test]
fn oversize_selection_falls_back_without_any_flash_write() {
    let data = include_bytes!("../../display/tests/fixtures/cache-oversize.zip");
    let flash = Memory::new();
    let c = settings();
    let (mut cached, result) = update(&flash, &Client::new(data), &c, NOW);
    result.unwrap();
    let mut direct = online(&Client::new(data), &c, NOW);
    assert_eq!(events(&mut cached), events(&mut direct));
    assert_eq!(flash.counters(), (0, 0));
    assert!(run(cache::load(&flash)).unwrap().is_none());
    update(&flash, &Client::new(data), &c, NOW + 1).1.unwrap();
    assert_eq!(flash.counters(), (0, 0));
}

#[test]
fn persisted_check_time_is_successful_completion_not_query_start() {
    let flash = Memory::new();
    let c = settings();
    flash.checked_at.set(Some(NOW + 10));
    assert_eq!(
        update(&flash, &Client::new(ORIGINAL), &c, NOW)
            .1
            .unwrap()
            .checked_at,
        Some(NOW + 10)
    );
    let bank = run(cache::load(&flash)).unwrap().unwrap();
    let mut b = Builder::new("20261017".parse().unwrap());
    let fresh = run(cache::restore(
        &flash,
        bank,
        &mut b,
        &mut zip::Workspace::new(),
        &c,
        NOW + 20,
    ))
    .unwrap();
    assert_eq!(fresh.checked_at, Some(NOW + 10));
}
#[test]
fn source_age_ignores_archives_no_longer_used_by_the_window() {
    const A: &str =
        "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610170128/mdb-982-202610170128.zip";
    const LIST: &str = "https://files.mobilitydatabase.org/mdb-982/mdb-982-202610170128/mdb-982-202610170128.zip https://files.mobilitydatabase.org/mdb-982/mdb-982-202610150044/mdb-982-202610150044.zip";
    let mut client = Client::new(ORIGINAL);
    client.listing = Some(LIST);
    client.alternate = Some((
        A,
        include_bytes!("../../display/tests/fixtures/cache-stamped.zip"),
    ));
    let flash = Memory::new();
    let c = settings();
    assert_eq!(
        update(&flash, &client, &c, NOW)
            .1
            .unwrap()
            .source_updated_at,
        None
    );
    let bank = run(cache::load(&flash)).unwrap().unwrap();
    let mut b = Builder::new("20261017".parse().unwrap());
    let fresh = run(cache::restore(
        &flash,
        bank,
        &mut b,
        &mut zip::Workspace::new(),
        &c,
        NOW + 3 * 86400,
    ))
    .unwrap();
    assert!(!b.events.is_empty());
    assert_eq!(fresh.source_updated_at, Some(1792200000));
}
#[test]
fn commit_waits_for_ui_lock_without_treating_contention_as_obsolete() {
    let flash = Memory::new();
    let c = settings();
    update(&flash, &Client::new(ORIGINAL), &c, NOW).1.unwrap();
    flash.pause_commit.set(true);
    let client = Client::new(CHANGED);
    let mut b = Builder::new("20261017".parse().unwrap());
    let mut ws = zip::Workspace::new();
    let http = cache::Http {
        inner: &client,
        flash: &flash,
        enabled: true,
    };
    let mut future = pin!(download::refresh(&http, &mut b, &mut ws, &c, NOW + 1));
    while !flash.commit_waiting.get() {
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    let before = restore(&flash, &c, NOW);
    assert!(!before.events.iter().any(|e| {
        before
            .headsigns
            .get(e.headsign)
            .unwrap()
            .contains("Changed")
    }));
    flash.pause_commit.set(false);
    run(future).unwrap();
    let after = restore(&flash, &c, NOW + 1);
    assert!(
        after
            .events
            .iter()
            .any(|e| after.headsigns.get(e.headsign).unwrap().contains("Changed"))
    );
}
#[test]
fn error_or_cancel_after_writing_the_final_range_keeps_the_old_bank() {
    let initial = Memory::new();
    let c = settings();
    update(&initial, &Client::new(ORIGINAL), &c, NOW).1.unwrap();
    let saved = initial.bytes.borrow().clone();
    let flash = Memory::from(&saved);
    let mut client = Client::new(CHANGED);
    client.fail_stop_request = Some(2);
    // The cache backend may safely retry through the normal streaming mirror;
    // the failed provisional bank must still remain uncommitted.
    update(&flash, &client, &c, NOW + 1).1.unwrap();
    assert!(client.stop_requests.get() >= 3);
    assert!(flash.writes.get() > 0);
    assert!(!restore(&flash, &c, NOW).events.is_empty());
    assert_eq!(
        &flash.bytes.borrow()[..cache::BANK_SIZE as usize],
        &saved[..cache::BANK_SIZE as usize]
    );
    let flash = Memory::from(&saved);
    let mut client = Client::new(CHANGED);
    client.cancel_stop_request = Some(2);
    let http = cache::Http {
        inner: &client,
        flash: &flash,
        enabled: true,
    };
    let mut b = Builder::new("20261017".parse().unwrap());
    let mut ws = zip::Workspace::new();
    {
        let mut future = pin!(download::refresh(&http, &mut b, &mut ws, &c, NOW + 1));
        while client.stop_requests.get() < 2 {
            assert!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
    }
    assert!(flash.writes.get() > 0);
    assert_eq!(
        flash.scratch_active.get(),
        0,
        "scratch released on cancellation"
    );
    let old = restore(&flash, &c, NOW);
    assert!(
        !old.events
            .iter()
            .any(|e| old.headsigns.get(e.headsign).unwrap().contains("Changed"))
    );
}

#[test]
fn transient_cache_reads_never_choose_a_bank_to_erase() {
    let base = Memory::new();
    let c = settings();
    update(&base, &Client::new(ORIGINAL), &c, NOW).1.unwrap();
    let saved = base.bytes.borrow().clone();
    for offset in [cache::OFFSET + 8, cache::OFFSET, cache::OFFSET + 4096] {
        let flash = Memory::from(&saved);
        flash.read_fault.set(Some(offset));
        // A successful ordinary mirror query may still publish changed data;
        // an unreadable cache is preserved without selecting a write target.
        update(&flash, &Client::new(CHANGED), &c, NOW + 1)
            .1
            .unwrap();
        assert_eq!(flash.counters(), (0, 0));
        assert_eq!(*flash.bytes.borrow(), saved);
        assert!(!restore(&flash, &c, NOW).events.is_empty());
    }
}

#[test]
fn active_query_overflow_is_rejected_before_any_flash_write() {
    let c = settings();
    let data = include_bytes!("../../display/tests/fixtures/cache-many.zip");
    for populated in [false, true] {
        let flash = Memory::new();
        if populated {
            update(&flash, &Client::new(ORIGINAL), &c, NOW).1.unwrap();
        }
        let saved = flash.bytes.borrow().clone();
        let before = flash.counters();
        for _ in 0..2 {
            assert!(
                update(&flash, &Client::new(data), &c, NOW + 3 * 86400)
                    .1
                    .is_err()
            );
            assert_eq!(flash.counters(), before);
            assert_eq!(*flash.bytes.borrow(), saved);
        }
    }
}

#[test]
fn bloom_false_positive_keeps_unselected_malformed_rows_out_of_typed_join() {
    let data = include_bytes!("../../display/tests/fixtures/cache-bloom.zip");
    let c = settings();
    let flash = Memory::new();
    let (mut cached, result) = update(&flash, &Client::new(data), &c, NOW);
    result.unwrap();
    let mut direct = online(&Client::new(data), &c, NOW);
    assert_eq!(events(&mut cached), events(&mut direct));
    let mut replay = restore(&flash, &c, NOW);
    assert_eq!(events(&mut replay), events(&mut direct));
    let before = flash.counters();
    update(&flash, &Client::new(data), &c, NOW + 1).1.unwrap();
    assert_eq!(flash.counters(), before);
}

#[test]
fn first_fill_scans_stop_times_once_and_unchanged_checks_need_no_scratch() {
    let flash = Memory::new();
    let c = settings();
    let first = Client::new(ORIGINAL);
    update(&flash, &first, &c, NOW).1.unwrap();
    assert_eq!(
        first.stop_requests.get(),
        1,
        "empty-cache preflight only checks metadata/trips; stop_times validates while writing"
    );
    assert_eq!(flash.scratch_calls.get(), 1);
    let before = flash.counters();
    flash.scratch_fails.set(true);
    let repeated = Client::new(ORIGINAL);
    update(&flash, &repeated, &c, NOW + 1).1.unwrap();
    assert_eq!(repeated.stop_requests.get(), 0);
    assert_eq!(flash.scratch_calls.get(), 1);
    assert_eq!(flash.counters(), before);
}

#[test]
fn first_fill_late_error_and_cancel_leave_no_committed_bank_and_can_retry() {
    let c = settings();
    for cancel in [false, true] {
        let flash = Memory::new();
        let mut client = Client::new(ORIGINAL);
        if cancel {
            client.cancel_stop_request = Some(1);
        } else {
            // Also fail any ordinary-query fallback after the cache attempt.
            client.late_error = true;
        }
        let http = cache::Http {
            inner: &client,
            flash: &flash,
            enabled: true,
        };
        let mut builder = Builder::new("20261017".parse().unwrap());
        let mut workspace = zip::Workspace::new();
        {
            let mut future = pin!(download::refresh(
                &http,
                &mut builder,
                &mut workspace,
                &c,
                NOW
            ));
            if cancel {
                while client.stop_requests.get() == 0 {
                    assert!(
                        future
                            .as_mut()
                            .poll(&mut Context::from_waker(Waker::noop()))
                            .is_pending()
                    );
                }
            } else {
                assert!(run(future).is_err());
            }
        }
        assert!(
            flash.writes.get() > 0,
            "the single pass writes provisional rows"
        );
        assert!(run(cache::load(&flash)).unwrap().is_none());
        assert!(
            builder.events.is_empty(),
            "failed stream discards provisional departures"
        );
        assert_eq!(flash.scratch_active.get(), 0);
        let retry = Client::new(ORIGINAL);
        let (mut cached, result) = update(&flash, &retry, &c, NOW);
        result.unwrap();
        assert_eq!(retry.stop_requests.get(), 1);
        let mut direct = online(&Client::new(ORIGINAL), &c, NOW);
        assert_eq!(events(&mut cached), events(&mut direct));
        assert!(run(cache::load(&flash)).unwrap().is_some());
    }
}

#[test]
fn scratch_allocation_failure_falls_back_without_flash_writes() {
    for metadata in [false, true] {
        let flash = Memory::new();
        let c = settings();
        update(&flash, &Client::new(ORIGINAL), &c, NOW).1.unwrap();
        let saved = flash.bytes.borrow().clone();
        let before = flash.counters();
        if metadata {
            flash.metadata_fails.set(true);
        } else {
            flash.scratch_fails.set(true);
        }
        let (mut cached, result) = update(&flash, &Client::new(CHANGED), &c, NOW + 1);
        result.unwrap();
        let mut direct = online(&Client::new(CHANGED), &c, NOW + 1);
        assert_eq!(events(&mut cached), events(&mut direct));
        assert_eq!(flash.counters(), before);
        assert_eq!(*flash.bytes.borrow(), saved);
    }
}

#[test]
fn saved_check_reference_replays_without_a_clock_or_flash_writes() {
    let flash = Memory::new();
    let c = settings();
    let (mut original, result) = update(&flash, &Client::new(ORIGINAL), &c, NOW);
    result.unwrap();
    let before = flash.counters();
    let bank = run(cache::load(&flash)).unwrap().unwrap();
    assert_eq!(bank.checked_at(), NOW);
    let mut builder = Builder::new("20261017".parse().unwrap());
    let fresh = run(cache::restore(
        &flash,
        bank,
        &mut builder,
        &mut zip::Workspace::new(),
        &c,
        bank.checked_at(),
    ))
    .unwrap();
    assert_eq!(events(&mut original), events(&mut builder));
    let mut app = timetable_display::App::default();
    app.publish(
        core::mem::take(&mut builder.events),
        core::mem::take(&mut builder.headsigns),
        builder.now,
    );
    app.freshness = fresh;
    assert_eq!(app.clock, None);
    assert_eq!(app.upcoming().count(), 0);
    assert!(app.board_departures().next().is_some());
    assert_eq!(flash.counters(), before);
}

#[test]
fn identical_selection_ids_from_different_networks_cannot_restore_each_others_data() {
    let flash = Memory::new();
    let c = settings();
    flash.identity.set(b"feed A / timezone rules A");
    update(&flash, &Client::new(ORIGINAL), &c, NOW).1.unwrap();
    let bank = run(cache::load(&flash)).unwrap().unwrap();
    flash.identity.set(b"feed B / timezone rules B");
    let mut builder = Builder::new("20261017".parse().unwrap());
    let mut ws = zip::Workspace::new();
    assert!(run(cache::restore(&flash, bank, &mut builder, &mut ws, &c, NOW)).is_err());
    assert!(builder.events.is_empty());
    flash.identity.set(b"feed A / timezone rules A");
    assert!(run(cache::restore(&flash, bank, &mut builder, &mut ws, &c, NOW)).is_ok());
}

#[test]
fn conditional_refresh_uses_no_archive_ranges_or_flash_writes_and_legacy_cache_still_restores() {
    let flash = Memory::new();
    let c = settings();
    let (mut original, result) = update(&flash, &Client::new(ORIGINAL), &c, NOW);
    result.unwrap();
    let expected = events(&mut original);
    let counters = flash.counters();
    let mut client = Client::new(ORIGINAL);
    client.conditional = true;
    let (mut fresh, result) = update(&flash, &client, &c, NOW + 60);
    assert_eq!(result.unwrap().checked_at, Some(NOW + 60));
    assert_eq!(events(&mut fresh), expected);
    assert_eq!(client.checks.get(), 1);
    assert_eq!(client.bytes.get(), 0);
    assert_eq!(flash.counters(), counters);
    // Convert only the header to the original v1 format. Payload offsets and
    // committed bytes are deliberately unchanged: upgrades must preserve it.
    {
        let mut bytes = flash.bytes.borrow_mut();
        assert_eq!(&bytes[..4], b"TTFC");
        let sources: Vec<_> = (0..3)
            .map(|i| bytes[48 + i * 352..48 + i * 352 + 160].to_vec())
            .collect();
        for (i, source) in sources.iter().enumerate() {
            bytes[48 + i * 160..48 + (i + 1) * 160].copy_from_slice(source);
        }
        bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());
        bytes[40..48].fill(0);
        let crc = crc32fast::hash(&bytes[..528]);
        bytes[40..44].copy_from_slice(&crc.to_le_bytes());
        bytes[44..48].copy_from_slice(&0x434f4d54_u32.to_le_bytes());
    }
    assert_eq!(events(&mut restore(&flash, &c, NOW)), expected);
    let client = Client::new(ORIGINAL);
    update(&flash, &client, &c, NOW + 120).1.unwrap();
    assert_eq!(client.checks.get(), 0);
    assert!(client.bytes.get() > 0);
    assert_eq!(flash.counters(), counters);
}

#[test]
fn validator_changes_in_directory_or_table_ranges_cannot_replace_the_validated_bank() {
    for drift in [1, 2] {
        let flash = Memory::new();
        let c = settings();
        update(&flash, &Client::new(ORIGINAL), &c, NOW).1.unwrap();
        let saved = flash.bytes.borrow().clone();
        let counters = flash.counters();
        let mut client = Client::new(CHANGED);
        client.drift = drift;
        let result = update(&flash, &client, &c, NOW + 1).1;
        assert!(
            result.is_err(),
            "A changed range validator must abort the cache update"
        );
        assert_eq!(flash.counters(), counters);
        assert_eq!(*flash.bytes.borrow(), saved);
        assert!(!events(&mut restore(&flash, &c, NOW)).is_empty());
    }
}

#[test]
fn weak_etag_sources_keep_normal_archive_validation_without_a_304_shortcut() {
    let flash = Memory::new();
    let c = settings();
    let mut weak = Client::new(ORIGINAL);
    weak.drift = 4;
    update(&flash, &weak, &c, NOW).1.unwrap();
    let counters = flash.counters();
    let mut refresh = Client::new(ORIGINAL);
    refresh.drift = 4;
    refresh.conditional = true;
    update(&flash, &refresh, &c, NOW + 60).1.unwrap();
    assert_eq!(refresh.checks.get(), 0);
    assert!(refresh.bytes.get() > 0);
    assert_eq!(flash.counters(), counters);
}
