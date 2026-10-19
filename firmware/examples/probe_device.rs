//! Actual firmware update with hard 64 KiB first-fit heap, radio baseline and
//! configurable host CPU duty. This is not ESP32 instruction/power emulation.
use core::{
    cell::Cell,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use embedded_io_async::{ErrorType, Read, Write};
use embedded_tls::{
    Aes128GcmSha256, Certificate, CryptoProvider, TlsClock, TlsConfig, TlsConnection, TlsContext,
    TlsVerifier, pki::CertVerifier,
};
use reqwless::{request::Method, response::Response};
use std::{
    net::TcpStream,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use timetable_display::{App, Config, catalog::Catalog, feed::Builder};
use timetable_firmware::{
    cooperative::Cooperative,
    download::{self, Error, Http, Metadata, Operation},
};
#[path = "support/budget.rs"]
mod budget;
#[path = "../src/http_request.rs"]
mod http_request;
#[path = "../src/tls_memory.rs"]
mod tls_memory;
#[global_allocator]
static ALLOC: budget::Allocator = budget::Allocator;
struct Clock;
impl TlsClock for Clock {
    fn now() -> Option<u64> {
        Some(SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs())
    }
}
struct Provider {
    rng: rand_core::OsRng,
    verifier: tls_memory::HeapCheckedVerifier<CertVerifier<'static, Aes128GcmSha256, Clock, 8192>>,
}
impl CryptoProvider for Provider {
    type CipherSuite = Aes128GcmSha256;
    type Signature = [u8; 64];
    fn rng(&mut self) -> impl embedded_tls::CryptoRngCore {
        &mut self.rng
    }
    fn verifier(
        &mut self,
    ) -> Result<&mut impl TlsVerifier<Aes128GcmSha256>, embedded_tls::TlsError> {
        Ok(&mut self.verifier)
    }
}
// Blocking host sockets make each I/O poll ready; Cooperative's explicit yields
// are resumed by run(). The device uses embassy-net and its normal executor.
struct Tcp(TcpStream);
impl ErrorType for Tcp {
    type Error = std::io::Error;
}
impl Read for Tcp {
    async fn read(&mut self, bytes: &mut [u8]) -> Result<usize, Self::Error> {
        std::io::Read::read(&mut self.0, bytes)
    }
}
impl Write for Tcp {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize, Self::Error> {
        std::io::Write::write(&mut self.0, bytes)
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        std::io::Write::flush(&mut self.0)
    }
}

fn cpu_time() -> Duration {
    let mut stamp = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut stamp) },
        0
    );
    Duration::new(stamp.tv_sec as u64, stamp.tv_nsec as u32)
}
fn run<T>(future: impl Future<Output = T>, duty: u32) -> T {
    let mut future = pin!(future);
    let started = Instant::now();
    let cpu = cpu_time();
    loop {
        let result = future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()));
        let target = (cpu_time() - cpu).mul_f64(100.0 / f64::from(duty));
        if let Some(delay) = target.checked_sub(started.elapsed()) {
            std::thread::sleep(delay);
        }
        if let Poll::Ready(value) = result {
            return value;
        }
    }
}
struct Client {
    bytes: Cell<u64>,
    requests: Cell<u32>,
    fallback: Cell<bool>,
    official_only: bool,
}
struct Counting<'a, R> {
    inner: R,
    client: &'a Client,
    bytes: u64,
    started: Instant,
    report: Instant,
}
impl<R: ErrorType> ErrorType for Counting<'_, R> {
    type Error = R::Error;
}
impl<R: Read> Read for Counting<'_, R> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let n = self.inner.read(out).await?;
        self.bytes += n as u64;
        self.client.bytes.set(self.client.bytes.get() + n as u64);
        if self.report.elapsed() >= Duration::from_secs(5) {
            println!(
                "progress: {} bytes / {}ms",
                self.bytes,
                self.started.elapsed().as_millis()
            );
            self.report = Instant::now();
        }
        Ok(n)
    }
}
impl Http for Client {
    fn stage(&self, message: &str) {
        println!("stage: {message}");
    }
    fn fallback(&self, error: &Error) {
        self.fallback.set(true);
        println!("FALLBACK: {error:?}; official ZTM");
    }
    async fn fetch(
        &self,
        url: &str,
        range: Option<(gtfs::http::ByteRange, Option<u64>)>,
        op: &mut impl Operation,
    ) -> Result<Metadata, Error> {
        if self.official_only && url == gtfs::poznan::MIRROR_PAGE {
            return Err(Error::Network);
        }
        let endpoint = http_request::endpoint(url)?;
        self.requests.set(self.requests.get() + 1);
        println!(
            "GET {url} range {range:?}; heap free {}",
            budget::available()
        );
        let socket = TcpStream::connect((endpoint.host, 443)).map_err(|_| Error::Network)?;
        socket
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let mut rx = [0; 16 * 1024 + 256];
        let mut tx = [0; http_request::TLS_TX_BYTES];
        let mut tls = TlsConnection::new(Tcp(socket), &mut rx, &mut tx);
        let config = TlsConfig::new()
            .with_server_name(endpoint.host)
            .enable_rsa_signatures();
        let mut provider = Provider {
            rng: rand_core::OsRng,
            verifier: tls_memory::HeapCheckedVerifier::new(
                CertVerifier::new(Certificate::X509(endpoint.root)),
                budget::available,
            )
            .with_redundant_root(endpoint.redundant_root),
        };
        tls.open(TlsContext::new(&config, &mut provider))
            .await
            .map_err(|e| {
                println!("TLS failed: {e:?}");
                Error::Tls
            })?;
        let mut stream = http_request::Stream(Cooperative::new(&mut tls));
        let mut value = [0; 64];
        if let Some((r, _)) = range {
            http_request::send_get_range(
                &mut stream,
                endpoint.path,
                endpoint.host,
                Some(r.header(&mut value)?),
            )
            .await
        } else {
            http_request::send_get(&mut stream, endpoint.path, endpoint.host).await
        }
        .map_err(|e| {
            println!("send failed: {e:?}");
            Error::Http
        })?;
        let mut header = [0; 2048];
        let response = Response::read(&mut stream, Method::GET, &mut header)
            .await
            .map_err(|e| {
                println!("response failed: {e:?}");
                Error::Http
            })?;
        let mut meta = http_request::validate(response.status.0, response.headers(), range)
            .map_err(|e| {
                println!("HTTP {} validation: {e:?}", response.status.0);
                e
            })?;
        meta.modified = timetable_display::freshness::source_modified(
            response.headers(),
            Clock::now().unwrap() as i64,
        );
        let started = Instant::now();
        let mut body = Counting {
            inner: response.body().reader(),
            client: self,
            bytes: 0,
            started,
            report: started,
        };
        op.read(&mut body).await?;
        println!(
            "body complete: {}bytes / {}ms",
            body.bytes,
            started.elapsed().as_millis()
        );
        Ok(meta)
    }
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let line = args.first().map_or("4", String::as_str);
    let stop = args.get(1).map_or("MT44", String::as_str);
    let duty: u32 = args
        .get(2)
        .map_or(Ok(20), |s| s.parse())
        .expect("CPU duty1..100");
    assert!((1..=100).contains(&duty));
    let catalog = Catalog::new(include_bytes!("../../catalog/catalog.bin")).unwrap();
    let route = (0..catalog.route_count())
        .find_map(|i| catalog.route(i).filter(|r| r.line == line))
        .expect("catalogue line");
    let platform = (0..catalog.stop_count())
        .find_map(|i| catalog.stop(i).filter(|s| s.code == stop))
        .expect("catalogue stop code");
    let mut image = *include_bytes!("../../display/tests/settings.bin");
    let mut config = timetable_display::config::decode(&mut image).unwrap().0;
    config.groups.truncate(1);
    config.groups[0].picks.truncate(1);
    let pick = &mut config.groups[0].picks[0];
    pick.route = route.id.try_into().unwrap();
    pick.line = route.line.try_into().unwrap();
    pick.stop = platform.id.try_into().unwrap();
    pick.stop_code = platform.code.try_into().unwrap();
    pick.stop_name = platform.name.try_into().unwrap();
    pick.direction = -1;
    let now = args.get(3).filter(|s| !s.starts_with("--")).map_or_else(
        || Clock::now().unwrap() as i64,
        |s| s.parse().expect("query Unix UTC"),
    );
    let mut builder = Builder::new("20261019".parse().unwrap());
    let mut workspace = gtfs::zip::Workspace::new();
    let client = Client {
        bytes: Cell::new(0),
        requests: Cell::new(0),
        fallback: Cell::new(false),
        official_only: args.iter().any(|s| s == "--official"),
    };
    let mut app = App::default();
    // Initialize stdout and entropy before switching to the bounded allocator.
    use rand_core::RngCore;
    rand_core::OsRng.fill_bytes(&mut [0; 8]);
    println!(
        "profile: heap65536B, radio baseline34320B, host CPU duty{duty}%; line {line}, stop {stop}"
    );
    let future = download::refresh(&client, &mut builder, &mut workspace, &config, now);
    println!(
        "host update future: {}B; Builder {}B; Workspace {}B; Config {}B",
        core::mem::size_of_val(&future),
        core::mem::size_of::<Builder>(),
        core::mem::size_of::<gtfs::zip::Workspace>(),
        core::mem::size_of::<Config>()
    );
    let reserve = budget::start(34320);
    let started = Instant::now();
    let cpu = cpu_time();
    let result = run(future, duty);
    let (peak, retained) = budget::finish(34320, reserve);
    println!(
        "result: {result:?}; requests{}; bytes{}; wall{}ms; CPU{}ms; heap peak{peak}/65536B; retained{retained}B; fallback{}",
        client.requests.get(),
        client.bytes.get(),
        started.elapsed().as_millis(),
        (cpu_time() - cpu).as_millis(),
        client.fallback.get()
    );
    assert_eq!(retained, 0, "unexpected retained allocation");
    let freshness = result.expect("validated device update");
    app.publish(
        core::mem::take(&mut builder.events),
        core::mem::take(&mut builder.headsigns),
        builder.now,
    );
    app.freshness = freshness;
    // Compare public values, not snapshot-dependent heading-pool indices.
    let mut checksum = 0xcbf29ce484222325_u64;
    for event in app.departures.iter() {
        for byte in event
            .at
            .to_le_bytes()
            .into_iter()
            .chain([event.group, event.pick, u8::from(event.conditional)])
            .chain(app.headsigns.get(event.headsign).unwrap_or("").bytes())
            .chain([0])
        {
            checksum = (checksum ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    println!(
        "published {} selected departures; checksum{checksum:016x}",
        app.departures.len()
    );
}
