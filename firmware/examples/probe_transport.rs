//! Host-only live diagnostic. Shares the device GET sender, stream adapter,
//! cooperative reader and certificate verifier; does not emulate the C3 radio.
use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use embedded_io_async::{ErrorType, Read, Write};
use embedded_tls::{
    Aes128GcmSha256, Certificate, CryptoProvider, TlsClock, TlsConfig, TlsConnection, TlsContext,
    TlsVerifier, pki::CertVerifier,
};
use reqwless::{
    request::{Method, Request, RequestBuilder},
    response::Response,
};
use std::{
    net::TcpStream,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use timetable_firmware::{cooperative::Cooperative, sources::Archives};
#[path = "../src/http_request.rs"]
mod http_request;
#[path = "../src/tls_memory.rs"]
mod tls_memory;
const HOST: &str = "www.ztm.poznan.pl";
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
fn run<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(value) = future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            return value;
        }
    }
}
trait Operation {
    async fn read<R: Read>(&mut self, body: R);
}
async fn fetch(
    path: &str,
    operation: &mut impl Operation,
    flush: bool,
) -> Result<(), reqwless::Error> {
    let started = Instant::now();
    let full_url = format!("https://{HOST}{path}");
    let endpoint =
        http_request::endpoint(&full_url).map_err(|_| reqwless::Error::ConnectionAborted)?;
    let socket = TcpStream::connect((HOST, 443)).expect("TCP connect");
    socket
        .set_read_timeout(Some(Duration::from_secs(if flush { 30 } else { 2 })))
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
        // Host admission is unconstrained. The separate offline tls_heap test
        // exercises the C3's budget with the actual first-fit allocator.
        verifier: tls_memory::HeapCheckedVerifier::new(
            CertVerifier::new(Certificate::X509(endpoint.root)),
            || usize::MAX,
        )
        .with_redundant_root(endpoint.redundant_root),
    };
    tls.open(TlsContext::new(&config, &mut provider))
        .await
        .expect("authenticated TLS");
    let mut stream = http_request::Stream(Cooperative::new(&mut tls));
    if flush {
        http_request::send_get(&mut stream, endpoint.path, endpoint.host).await?;
    } else {
        // Intentionally reproduce the previous device path for comparison.
        Request::get(path)
            .host(HOST)
            .headers(&[("Accept-Encoding", "identity"), ("Connection", "close")])
            .build()
            .write_header(&mut stream)
            .await?;
    }
    let mut headers = [0; 2048];
    let response = Response::read(&mut stream, Method::GET, &mut headers).await?;
    http_request::validate(response.status.0, response.headers(), None)
        .map_err(|_| reqwless::Error::ConnectionAborted)?;
    operation.read(response.body().reader()).await;
    println!(
        "HTTP 200 and complete body: {path} ({:.3} s)",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
struct Listing {
    count: usize,
    selected: Archives,
}
impl Operation for Listing {
    async fn read<R: Read>(&mut self, body: R) {
        let mut listing = gtfs::poznan::Archives::new(body);
        while let Some(name) = listing.next().await.expect("archive listing stream") {
            self.count += 1;
            self.selected.consider(name);
        }
    }
}
struct Browse;
impl Operation for Browse {
    async fn read<R: Read>(&mut self, body: R) {
        let mut workspace = gtfs::zip::Workspace::new();
        let (mut routes, mut stops) = (0, 0);
        let manifest = gtfs::stream::scan(
            body,
            &[gtfs::Table::Routes, gtfs::Table::Stops],
            None,
            &mut workspace,
            |row| {
                match row.table() {
                    gtfs::Table::Routes => routes += 1,
                    gtfs::Table::Stops => stops += 1,
                    _ => unreachable!(),
                }
                Ok(())
            },
        )
        .await
        .expect("complete ZIP, selected table CRCs, directory and EOF");
        assert!(routes > 0 && stops > 0);
        println!(
            "Validated ZIP: {routes} routes, {stops} stops, source UTC {:?}",
            manifest.updated_at()
        );
    }
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let date: gtfs::Date = args
        .first()
        .expect("usage: probe_transport YYYYMMDD [--without-flush]")
        .parse()
        .expect("YYYYMMDD");
    let flush = !args.iter().any(|arg| arg == "--without-flush");
    let mut listing = Listing {
        count: 0,
        selected: Archives::new([date; 3], 1),
    };
    let result = run(fetch("/otwarte-dane/gtfsfiles/", &mut listing, flush));
    if !flush {
        assert!(
            result.is_err(),
            "unflushed request unexpectedly received a response"
        );
        println!("Previous unflushed path failed as expected: {result:?}");
        return;
    }
    result.expect("HTTP listing");
    let name = listing
        .selected
        .sources()
        .expect("no official archive covers this date")[0]
        .0
        .unwrap();
    println!(
        "Discovered {} archive links; selected {}",
        listing.count,
        name.as_str()
    );
    let mut url = [0; 192];
    let path = name
        .url(&mut url)
        .unwrap()
        .strip_prefix("https://www.ztm.poznan.pl")
        .unwrap();
    run(fetch(path, &mut Browse, true)).expect("HTTP ZIP stream");
}
