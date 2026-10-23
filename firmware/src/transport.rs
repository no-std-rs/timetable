//! Verified TLS 1.3 over embassy-net; reqwless decodes each streaming HTTP body.
use crate::http_session::{Connection, Observe};
use crate::tls_cipher::CipherSuite;
use crate::tls_memory::HeapCheckedVerifier;
use embassy_net::{
    Stack,
    dns::DnsQueryType,
    tcp::TcpSocket,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_time::{Duration, Instant, with_timeout};
use embedded_tls::{
    Certificate, CryptoProvider, TlsClock, TlsConfig, TlsConnection, TlsContext, TlsVerifier,
    pki::CertVerifier,
};
use esp_hal::rng::{Rng, Trng};
use gtfs::http::ByteRange;
use portable_atomic::{AtomicI64, Ordering};
use timetable_firmware::cooperative::Cooperative;
pub use timetable_firmware::download::Error;
use timetable_firmware::download::Operation;
use timetable_firmware::download::{self, Metadata, SessionOperation};

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
    verifier: HeapCheckedVerifier<CertVerifier<'static, CipherSuite, Clock, 8192>>,
}
impl CryptoProvider for Provider {
    type CipherSuite = CipherSuite;
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
    fn feed_id(&self) -> &str {
        crate::catalog_slot::profile().feed_id().unwrap()
    }
    fn stage(&self, message: &str) {
        log::info!("Feed: {}", message);
    }
    fn fallback(&self, error: &Error) {
        log::warn!(
            "Feed: MobilityDatabase failed: {:?}; using official ZTM fallback",
            error
        );
    }
    async fn mirror_tables(
        &self,
        snapshots: &download::Snapshots,
        builder: &mut timetable_display::feed::Builder,
        workspace: &mut gtfs::zip::Workspace,
        config: &timetable_display::Config,
    ) -> Result<timetable_display::freshness::Freshness, Error> {
        self.session(
            "https://files.mobilitydatabase.org/",
            &mut download::MirrorQuery {
                snapshots,
                builder,
                workspace,
                config,
            },
            true,
        )
        .await
    }
    async fn mirror_cached_tables(
        &self,
        snapshots: &download::Snapshots,
        builder: &mut timetable_display::feed::Builder,
        workspace: &mut gtfs::zip::Workspace,
        config: &timetable_display::Config,
        flash: &impl timetable_firmware::cache::Storage,
    ) -> Result<timetable_display::freshness::Freshness, Error> {
        self.session(
            "https://files.mobilitydatabase.org/",
            &mut timetable_firmware::cache::Query {
                snapshots,
                builder,
                workspace,
                config,
                flash,
            },
            true,
        )
        .await
    }
    async fn fetch(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        operation: &mut impl Operation,
    ) -> Result<Metadata, Error> {
        with_timeout(
            Duration::from_secs(300),
            self.session(
                url,
                &mut download::Fetch {
                    url,
                    range,
                    operation,
                },
                false,
            ),
        )
        .await
        .map_err(|_| Error::Network)?
    }
}
impl Http<'_> {
    async fn session<S: SessionOperation>(
        &self,
        url: &str,
        operation: &mut S,
        persistent: bool,
    ) -> Result<S::Output, Error> {
        if now().is_none() {
            return Err(Error::Clock);
        }
        let endpoint = crate::http_request::endpoint(url)?;
        let host = endpoint.host;
        let addresses = self
            .0
            .dns_query(host, DnsQueryType::A)
            .await
            .map_err(|_| Error::Network)?;
        let ip = *addresses.first().ok_or(Error::Network)?;
        let mut rx = [0; 6144];
        let mut tx = [0; 1024];
        let mut socket = TcpSocket::new(self.0, &mut rx, &mut tx);
        socket.set_timeout(Some(Duration::from_secs(30)));
        socket.set_nagle_enabled(false);
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
            with_timeout(
                Duration::from_secs(60),
                tls.open(TlsContext::new(&tls_config, &mut provider)),
            )
            .await
            .map_err(|_| Error::Tls)?
            .map_err(|error| {
                log::warn!(
                    "TLS handshake failed: {:?}; {} B heap free",
                    error,
                    esp_alloc::HEAP.free()
                );
                Error::Tls
            })?;
        }
        log::info!("TLS: handshake verified; ChaCha20-Poly1305; TCP RX 6144 B");
        crate::log_heap();
        log::info!("HTTP: opened persistent session for {}", host);
        let stream = crate::http_request::Stream(Cooperative::new(&mut tls));
        let connection = Timed(Connection::new(stream, host, &Device, persistent));
        let mut query = core::pin::pin!(operation.run(&connection));
        let result = crate::scheduler::application(query.as_mut()).await;
        crate::scheduler::report();
        result
    }
}
struct Timed<H>(H);
impl<H: download::Http> download::Http for Timed<H> {
    async fn unchanged(&self, url: &str, validator: &download::Validator) -> Result<bool, Error> {
        with_timeout(Duration::from_secs(300), self.0.unchanged(url, validator))
            .await
            .map_err(|_| Error::Network)?
    }
    fn stage(&self, message: &str) {
        self.0.stage(message);
    }
    async fn fetch(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        operation: &mut impl Operation,
    ) -> Result<Metadata, Error> {
        with_timeout(
            Duration::from_secs(300),
            self.0.fetch(url, range, operation),
        )
        .await
        .map_err(|_| Error::Network)?
    }
}
struct Device;
impl Observe for Device {
    fn millis(&self) -> u64 {
        Instant::now().as_millis()
    }
    fn now(&self) -> Option<i64> {
        now()
    }
    fn stage(&self, message: &str) {
        log::info!("Feed: {}", message);
    }
    fn request(&self, url: &str, range: Option<(ByteRange, Option<u64>)>) {
        log::info!(
            "HTTP: requesting {} range {:?}; reusing verified TLS",
            url,
            range
        );
    }
    fn status(&self, status: u16) {
        log::info!("HTTP: response status {}", status);
    }
    fn progress(&self, bytes: u64, total: Option<u64>, elapsed: u64) {
        crate::scheduler::report();
        let seconds = (elapsed / 1000).max(1);
        if let Some(total) = total.filter(|n| *n != 0) {
            log::info!(
                "HTTP: {} / {} KiB ({}%); {} KiB/s; {} s elapsed",
                bytes / 1024,
                total / 1024,
                bytes.saturating_mul(100) / total,
                bytes / seconds / 1024,
                seconds
            );
        } else {
            log::info!(
                "HTTP: {} KiB; {} KiB/s; {} s elapsed",
                bytes / 1024,
                bytes / seconds / 1024,
                seconds
            );
        }
    }
    fn complete(&self, bytes: u64, elapsed: u64) {
        log::info!("HTTP: body complete; {} bytes in {} ms", bytes, elapsed);
        if elapsed >= 1000 {
            crate::scheduler::report();
        }
    }
}

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
