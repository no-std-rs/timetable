use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use embedded_io_async::{ErrorType, Read};
use gtfs::{http::ByteRange, zip};
use std::cell::RefCell;
use timetable_display::{clock::Zone, feed::Builder};
use timetable_firmware::download::{self, Error, Http, Metadata, Operation};

struct Body<'a>(&'a [u8]);
impl ErrorType for Body<'_> {
    type Error = embedded_io_async::ErrorKind;
}
impl Read for Body<'_> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let n = out.len().min(self.0.len()).min(13);
        out[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        Ok(n)
    }
}
fn run<T>(f: impl Future<Output = T>) -> T {
    let mut f = pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            return v;
        }
    }
}
struct World {
    offline: bool,
    calls: RefCell<Vec<String>>,
}
impl Http for World {
    fn feed_id(&self) -> &str {
        "demo-ny"
    }
    async fn fetch(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        op: &mut impl Operation,
    ) -> Result<Metadata, Error> {
        self.calls.borrow_mut().push(url.into());
        if self.offline {
            return Err(Error::Network);
        }
        if url == "https://mobilitydatabase.org/feeds/gtfs/demo-ny" {
            op.read(Body(b"https://files.mobilitydatabase.org/demo-ny/demo-ny-202610170128/demo-ny-202610170128.zip")).await?;
            return Ok(Metadata::default());
        }
        assert!(url.starts_with("https://files.mobilitydatabase.org/demo-ny/"));
        let bytes = include_bytes!("../../web/tests/fixtures/long-line.zip");
        let (range, size) = range.expect("no full ZIP transfer");
        assert!(size.is_none_or(|n| n == bytes.len() as u64));
        op.read(Body(&bytes[range.start() as usize..=range.end() as usize]))
            .await?;
        Ok(Metadata {
            length: Some(bytes.len() as u64),
            modified: None,
            ..Metadata::default()
        })
    }
}
#[test]
fn installed_world_selection_uses_its_timezone_optional_feed_info_and_bounded_line_label() {
    let mut image = *include_bytes!("../../display/tests/settings.bin");
    let mut c = timetable_display::config::decode(&mut image).unwrap().0;
    c.groups.truncate(1);
    c.groups[0].picks.truncate(1);
    let p = &mut c.groups[0].picks[0];
    p.route = "R".try_into().unwrap();
    p.line = "ABCDE...".try_into().unwrap();
    p.stop = "S1".try_into().unwrap();
    p.stop_code = "P1".try_into().unwrap();
    p.direction = 0;
    let mut zone = Vec::new();
    zone.extend_from_slice(&1704067200i64.to_le_bytes());
    zone.extend_from_slice(&1893456000i64.to_le_bytes());
    zone.extend_from_slice(&(-14400i32).to_le_bytes());
    let mut b = Builder::new("20261021".parse().unwrap());
    b.zone = Zone::from_bytes(Box::leak(zone.into_boxed_slice()));
    let now = 1792584000; // 2026-10-21 12:00 UTC = 08:00 New York.
    let mut ws = zip::Workspace::new();
    let http = World {
        offline: false,
        calls: RefCell::new(Vec::new()),
    };
    run(download::refresh(&http, &mut b, &mut ws, &c, now)).unwrap();
    assert_eq!(b.events.len(), 2);
    assert_eq!(b.events.iter().map(|e| e.at).min(), Some(now + 3600));
    let offline = World {
        offline: true,
        calls: RefCell::new(Vec::new()),
    };
    assert!(run(download::refresh(&offline, &mut b, &mut ws, &c, now)).is_err());
    assert_eq!(
        offline.calls.borrow().as_slice(),
        ["https://mobilitydatabase.org/feeds/gtfs/demo-ny"]
    );
    assert!(b.events.is_empty());
}
