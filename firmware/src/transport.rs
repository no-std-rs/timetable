//! Verified TLS 1.3 over embassy-net; reqwless decodes each streaming HTTP body.
use crate::tls_memory::HeapCheckedVerifier;
use embassy_net::{
    Stack,
    dns::DnsQueryType,
    tcp::TcpSocket,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_time::{Duration, Instant, with_timeout};
use embedded_io_async::{ErrorType, Read};
use embedded_tls::{
    Aes128GcmSha256, Certificate, CryptoProvider, TlsClock, TlsConfig, TlsConnection, TlsContext,
    TlsVerifier, pki::CertVerifier,
};
use esp_hal::rng::{Rng, Trng};
use gtfs::http::ByteRange;
use portable_atomic::{AtomicI64, Ordering};
use reqwless::{request::Method, response::Response};
use timetable_firmware::cooperative::Cooperative;
pub use timetable_firmware::download::Error;
use timetable_firmware::download::Operation;
use timetable_firmware::download::{self, Metadata};

static UTC_OFFSET: AtomicI64 = AtomicI64::new(i64::MIN);

pub fn now() -> Option<i64> {
    now_micros().map(|us| us.div_euclid(1_000_000))
}
pub fn now_micros() -> Option<i64> {
    let offset = UTC_OFFSET.load(Ordering::Acquire);
    (offset != i64::MIN).then(|| offset + Instant::now().as_micros() as i64)
}
/// Restore the RTC-derived clock before using cached departure times or TLS.
pub fn restore_clock(utc_micros: i64) {
    UTC_OFFSET.store(
        utc_micros - Instant::now().as_micros() as i64,
        Ordering::Release,
    );
}
struct Clock;
impl TlsClock for Clock {
    fn now() -> Option<u64> {
        now()?.try_into().ok()
    }
}

struct Provider {
    rng: Trng,
    verifier: HeapCheckedVerifier<CertVerifier<'static, Aes128GcmSha256, Clock, 8192>>,
}
impl CryptoProvider for Provider {
    type CipherSuite = Aes128GcmSha256;
    type Signature = [u8; 64];
    fn rng(&mut self) -> impl embedded_tls::CryptoRngCore {
        &mut self.rng
    }
    fn verifier(
        &mut self,
    ) -> Result<&mut impl TlsVerifier<Self::CipherSuite>, embedded_tls::TlsError> {
        Ok(&mut self.verifier)
    }
}
pub struct Http<'d>(pub Stack<'d>);
impl download::Http for Http<'_> {
    fn stage(&self, message: &str) {
        log::info!("Feed: {}", message);
    }
    fn fallback(&self, error: &Error) {
        log::warn!(
            "Feed: MobilityDatabase failed: {:?}; using official ZTM fallback",
            error
        );
    }
    async fn fetch(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        operation: &mut impl Operation,
    ) -> Result<Metadata, Error> {
        with_timeout(
            Duration::from_secs(300),
            self.request(url, range, operation),
        )
        .await
        .map_err(|_| Error::Network)?
    }
}
impl Http<'_> {
    async fn request(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        operation: &mut impl Operation,
    ) -> Result<Metadata, Error> {
        if now().is_none() {
            return Err(Error::Clock);
        }
        let endpoint = crate::http_request::endpoint(url)?;
        let (host, path) = (endpoint.host, endpoint.path);
        let addresses = self
            .0
            .dns_query(host, DnsQueryType::A)
            .await
            .map_err(|_| Error::Network)?;
        let ip = *addresses.first().ok_or(Error::Network)?;
        let mut rx = [0; 4096];
        let mut tx = [0; 1024];
        let mut socket = TcpSocket::new(self.0, &mut rx, &mut tx);
        socket.set_timeout(Some(Duration::from_secs(30)));
        socket
            .connect((ip, 443))
            .await
            .map_err(|_| Error::Network)?;
        let mut tls_rx = [0; 16 * 1024 + 256];
        let mut tls_tx = [0; crate::http_request::TLS_TX_BYTES];
        let mut tls = TlsConnection::new(socket, &mut tls_rx, &mut tls_tx);
        log::info!(
            "TLS: starting verified handshake; {} B heap free",
            esp_alloc::HEAP.free()
        );
        {
            let tls_config = TlsConfig::new()
                .with_server_name(host)
                .enable_rsa_signatures();
            let mut provider = Provider {
                rng: Trng::try_new().map_err(|_| Error::Tls)?,
                verifier: HeapCheckedVerifier::new(
                    CertVerifier::new(Certificate::X509(endpoint.root)),
                    || esp_alloc::HEAP.free(),
                )
                .with_redundant_root(endpoint.redundant_root),
            };
            // Keep the 8 KiB verifier out of owned async-context copies.
            tls.open(TlsContext::new(&tls_config, &mut provider))
                .await
                .map_err(|error| {
                    log::warn!(
                        "TLS handshake failed: {:?}; {} B heap free",
                        error,
                        esp_alloc::HEAP.free()
                    );
                    Error::Tls
                })?;
        }
        log::info!("TLS: handshake verified");
        crate::log_heap();
        // Keep the live TLS connection in place while reqwless borrows it.
        let mut stream = crate::http_request::Stream(Cooperative::new(&mut tls));
        let mut headers = [0; 2048];
        let mut range_text = [0; 64];
        let range_header = range.map(|(r, _)| r.header(&mut range_text)).transpose()?;
        log::info!(
            "HTTP: requesting https://{}{} range {:?}",
            host,
            path,
            range
        );
        let sent = if range_header.is_some() {
            crate::http_request::send_get_range(&mut stream, path, host, range_header).await
        } else {
            crate::http_request::send_get(&mut stream, path, host).await
        };
        sent.map_err(|error| {
            log::warn!("HTTP request write/flush failed: {:?}", error);
            Error::Http
        })?;
        log::info!("HTTP: request sent; reading response headers");
        let response = Response::read(&mut stream, Method::GET, &mut headers)
            .await
            .map_err(|error| {
                log::warn!("HTTP response headers failed: {:?}", error);
                Error::Http
            })?;
        log::info!("HTTP: response status {}", response.status.0);
        let mut meta = crate::http_request::validate(response.status.0, response.headers(), range)?;
        let modified = timetable_display::freshness::source_modified(
            response.headers(),
            now().ok_or(Error::Clock)?,
        );
        meta.modified = modified;
        let started = Instant::now();
        let mut body = Progress {
            inner: response.body().reader(),
            bytes: 0,
            expected: range.map(|(r, _)| r.len()).or(meta.length),
            started,
            report: started,
        };
        operation.read(&mut body).await?;
        log::info!(
            "HTTP: body complete; {} bytes in {} ms",
            body.bytes,
            started.elapsed().as_millis()
        );
        Ok(meta)
    }
}

struct Progress<R> {
    inner: R,
    bytes: u64,
    expected: Option<u64>,
    started: Instant,
    report: Instant,
}
impl<R: ErrorType> ErrorType for Progress<R> {
    type Error = R::Error;
}
impl<R: Read> Read for Progress<R> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let n = self.inner.read(out).await?;
        self.bytes += n as u64;
        if self.report.elapsed() >= Duration::from_secs(5) {
            let seconds = self.started.elapsed().as_secs().max(1);
            if let Some(total) = self.expected.filter(|n| *n != 0) {
                log::info!(
                    "HTTP: {} / {} KiB ({}%); {} KiB/s; {} s elapsed",
                    self.bytes / 1024,
                    total / 1024,
                    self.bytes.saturating_mul(100) / total,
                    self.bytes / 1024 / seconds,
                    seconds
                );
            } else {
                log::info!(
                    "HTTP: {} KiB; {} KiB/s; {} s elapsed",
                    self.bytes / 1024,
                    self.bytes / 1024 / seconds,
                    seconds
                );
            }
            self.report = Instant::now();
        }
        Ok(n)
    }
}

/// A nonce-bound SNTP reply supplies UTC before certificate validity is checked.
pub async fn synchronize(stack: Stack<'_>) -> Result<(), Error> {
    log::info!("Clock sync: resolving NTP server");
    with_timeout(Duration::from_secs(15), async {
        let addresses = stack
            .dns_query("pool.ntp.org", DnsQueryType::A)
            .await
            .map_err(|_| clock_failure("NTP DNS lookup failed"))?;
        let ip = *addresses.first().ok_or(Error::Clock)?;
        let mut rx_meta = [PacketMetadata::EMPTY; 1];
        let mut tx_meta = [PacketMetadata::EMPTY; 1];
        let mut rx = [0; 128];
        let mut tx = [0; 128];
        let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx, &mut tx_meta, &mut tx);
        socket
            .bind(44444)
            .map_err(|_| clock_failure("NTP socket bind failed"))?;
        let mut request = [0; 48];
        request[0] = 0x23;
        Rng::new().read(&mut request[40..48]);
        socket
            .send_to(&request, (ip, 123))
            .await
            .map_err(|_| clock_failure("NTP request send failed"))?;
        log::info!("Clock sync: NTP request sent; waiting for reply");
        let mut response = [0; 128];
        let (n, remote) = socket
            .recv_from(&mut response)
            .await
            .map_err(|_| clock_failure("NTP reply read failed"))?;
        if n < 48
            || remote.endpoint.addr != ip
            || remote.endpoint.port != 123
            || response[0] & 7 != 4
            || response[0] >> 6 == 3
            || !(1..=15).contains(&response[1])
            || response[24..32] != request[40..48]
        {
            return Err(clock_failure("NTP reply failed validation"));
        }
        let seconds = u32::from_be_bytes(response[40..44].try_into().unwrap()) as u64;
        let era = if seconds < 2_208_988_800 {
            1u64 << 32
        } else {
            0
        };
        let unix = (seconds + era - 2_208_988_800) as i64;
        if !(1_704_067_200..4_102_444_800).contains(&unix) {
            return Err(clock_failure("NTP time outside supported range"));
        }
        let fraction = u32::from_be_bytes(response[44..48].try_into().unwrap());
        let micros = unix * 1_000_000 + ((u64::from(fraction) * 1_000_000) >> 32) as i64;
        restore_clock(micros);
        Ok(())
    })
    .await
    .map_err(|_| clock_failure("15 second NTP timeout"))?
}

fn clock_failure(stage: &str) -> Error {
    log::warn!("Clock sync: {}", stage);
    Error::Clock
}
