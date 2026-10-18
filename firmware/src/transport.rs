//! Verified TLS 1.3 over embassy-net; reqwless decodes each streaming HTTP body.
use crate::tls_memory::HeapCheckedVerifier;
use embassy_net::{
    Stack,
    dns::DnsQueryType,
    tcp::TcpSocket,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_time::{Duration, Instant, with_timeout};
use embedded_io_async::Read;
use embedded_tls::{
    Aes128GcmSha256, Certificate, CryptoProvider, TlsClock, TlsConfig, TlsConnection, TlsContext,
    TlsVerifier, pki::CertVerifier,
};
use esp_hal::rng::{Rng, Trng};
use portable_atomic::{AtomicI64, Ordering};
use reqwless::{request::Method, response::Response};
use timetable_firmware::cooperative::Cooperative;

const HOST: &str = "www.ztm.poznan.pl";
static UTC_OFFSET: AtomicI64 = AtomicI64::new(i64::MIN);

#[derive(Debug)]
pub enum Error {
    Network,
    Tls,
    Http,
    Clock,
    MissingArchive,
    Feed(gtfs::Error),
}
impl From<gtfs::Error> for Error {
    fn from(e: gtfs::Error) -> Self {
        Self::Feed(e)
    }
}

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
pub trait Operation {
    async fn read<R: Read>(&mut self, body: R) -> Result<(), Error>;
}

pub struct Http(pub Stack<'static>);
impl Http {
    pub async fn get(
        &self,
        url: &str,
        operation: &mut impl Operation,
    ) -> Result<Option<i64>, Error> {
        with_timeout(Duration::from_secs(300), self.request(url, operation))
            .await
            .map_err(|_| Error::Network)?
    }
    async fn request(
        &self,
        url: &str,
        operation: &mut impl Operation,
    ) -> Result<Option<i64>, Error> {
        if now().is_none() {
            return Err(Error::Clock);
        }
        let path = url
            .strip_prefix("https://www.ztm.poznan.pl")
            .filter(|p| p.starts_with('/'))
            .ok_or(Error::Http)?;
        let addresses = self
            .0
            .dns_query(HOST, DnsQueryType::A)
            .await
            .map_err(|_| Error::Network)?;
        let ip = *addresses.first().ok_or(Error::Network)?;
        let mut rx = [0; 4096];
        let mut tx = [0; 2048];
        let mut socket = TcpSocket::new(self.0, &mut rx, &mut tx);
        socket.set_timeout(Some(Duration::from_secs(30)));
        socket
            .connect((ip, 443))
            .await
            .map_err(|_| Error::Network)?;
        let mut tls_rx = [0; 16 * 1024 + 256];
        let mut tls_tx = [0; 2048 + 256];
        let mut tls = TlsConnection::new(socket, &mut tls_rx, &mut tls_tx);
        log::info!(
            "TLS: starting verified handshake; {} B heap free",
            esp_alloc::HEAP.free()
        );
        {
            let tls_config = TlsConfig::new()
                .with_server_name(HOST)
                .enable_rsa_signatures();
            let mut provider = Provider {
                rng: Trng::try_new().map_err(|_| Error::Tls)?,
                verifier: HeapCheckedVerifier::new(
                    CertVerifier::new(Certificate::X509(include_bytes!(
                        "../certs/certum-trusted-root.der"
                    ))),
                    || esp_alloc::HEAP.free(),
                ),
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
        log::info!("HTTP: requesting {}", path);
        crate::http_request::send_get(&mut stream, path, HOST)
            .await
            .map_err(|error| {
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
        if response.status.0 != 200 {
            return Err(Error::Http);
        }
        for (name, value) in response.headers() {
            if name.eq_ignore_ascii_case("content-encoding") && value != b"identity" {
                log::warn!("HTTP: compressed response rejected");
                return Err(Error::Http);
            }
        }
        let modified = timetable_display::freshness::source_modified(
            response.headers(),
            now().ok_or(Error::Clock)?,
        );
        operation.read(response.body().reader()).await?;
        Ok(modified)
    }
}

/// A nonce-bound SNTP reply supplies UTC before certificate validity is checked.
pub async fn synchronize(stack: Stack<'static>) -> Result<(), Error> {
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
