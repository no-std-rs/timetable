//! Verified TLS 1.3 over embassy-net; reqwless decodes each streaming HTTP body.
use embassy_net::{
    Stack,
    dns::DnsQueryType,
    tcp::TcpSocket,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_time::{Duration, Instant, with_timeout};
use embedded_io_async::{ErrorType, Read, Write};
use embedded_tls::{
    Aes128GcmSha256, Certificate, CryptoProvider, TlsClock, TlsConfig, TlsConnection, TlsContext,
    TlsVerifier, pki::CertVerifier,
};
use esp_hal::rng::{Rng, Trng};
use portable_atomic::{AtomicI64, Ordering};
use reqwless::{
    TryBufRead,
    request::{Method, Request, RequestBuilder},
    response::Response,
};

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
    let offset = UTC_OFFSET.load(Ordering::Acquire);
    (offset != i64::MIN).then(|| offset + Instant::now().as_secs() as i64)
}
struct Clock;
impl TlsClock for Clock {
    fn now() -> Option<u64> {
        now()?.try_into().ok()
    }
}

struct Provider {
    rng: Trng,
    verifier: CertVerifier<'static, Aes128GcmSha256, Clock, 8192>,
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
struct Io<S>(S);
impl<S: ErrorType> ErrorType for Io<S> {
    type Error = S::Error;
}
impl<S: Read> Read for Io<S> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(out).await
    }
}
impl<S: Write> Write for Io<S> {
    async fn write(&mut self, data: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(data).await
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}
impl<S: Read> TryBufRead for Io<S> {}

pub trait Operation {
    async fn read<R: Read>(&mut self, body: R) -> Result<(), Error>;
}

pub struct Http(pub Stack<'static>);
impl Http {
    pub async fn get(&self, url: &str, operation: &mut impl Operation) -> Result<(), Error> {
        with_timeout(Duration::from_secs(300), self.request(url, operation))
            .await
            .map_err(|_| Error::Network)?
    }
    async fn request(&self, url: &str, operation: &mut impl Operation) -> Result<(), Error> {
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
        let tls_config = TlsConfig::new()
            .with_server_name(HOST)
            .enable_rsa_signatures();
        let mut tls = TlsConnection::new(socket, &mut tls_rx, &mut tls_tx);
        let provider = Provider {
            rng: Trng::try_new().map_err(|_| Error::Tls)?,
            verifier: CertVerifier::new(Certificate::X509(include_bytes!(
                "../certs/certum-trusted-root.der"
            ))),
        };
        tls.open(TlsContext::new(&tls_config, provider))
            .await
            .map_err(|_| Error::Tls)?;
        let mut stream = Io(tls);
        let mut headers = [0; 2048];
        Request::get(path)
            .host(HOST)
            .headers(&[("Accept-Encoding", "identity"), ("Connection", "close")])
            .build()
            .write_header(&mut stream)
            .await
            .map_err(|_| Error::Http)?;
        let response = Response::read(&mut stream, Method::GET, &mut headers)
            .await
            .map_err(|_| Error::Http)?;
        if response.status.0 != 200 {
            return Err(Error::Http);
        }
        for (name, value) in response.headers() {
            if name.eq_ignore_ascii_case("content-encoding") && value != b"identity" {
                return Err(Error::Http);
            }
        }
        operation.read(response.body().reader()).await
    }
}

/// A nonce-bound SNTP reply supplies UTC before certificate validity is checked.
pub async fn synchronize(stack: Stack<'static>) -> Result<(), Error> {
    with_timeout(Duration::from_secs(15), async {
        let addresses = stack
            .dns_query("pool.ntp.org", DnsQueryType::A)
            .await
            .map_err(|_| Error::Clock)?;
        let ip = *addresses.first().ok_or(Error::Clock)?;
        let mut rx_meta = [PacketMetadata::EMPTY; 1];
        let mut tx_meta = [PacketMetadata::EMPTY; 1];
        let mut rx = [0; 128];
        let mut tx = [0; 128];
        let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx, &mut tx_meta, &mut tx);
        socket.bind(44444).map_err(|_| Error::Clock)?;
        let mut request = [0; 48];
        request[0] = 0x23;
        Rng::new().read(&mut request[40..48]);
        socket
            .send_to(&request, (ip, 123))
            .await
            .map_err(|_| Error::Clock)?;
        let mut response = [0; 128];
        let (n, remote) = socket
            .recv_from(&mut response)
            .await
            .map_err(|_| Error::Clock)?;
        if n < 48
            || remote.endpoint.addr != ip
            || remote.endpoint.port != 123
            || response[0] & 7 != 4
            || response[0] >> 6 == 3
            || !(1..=15).contains(&response[1])
            || response[24..32] != request[40..48]
        {
            return Err(Error::Clock);
        }
        let seconds = u32::from_be_bytes(response[40..44].try_into().unwrap()) as u64;
        let era = if seconds < 2_208_988_800 {
            1u64 << 32
        } else {
            0
        };
        let unix = (seconds + era - 2_208_988_800) as i64;
        if !(1_704_067_200..4_102_444_800).contains(&unix) {
            return Err(Error::Clock);
        }
        UTC_OFFSET.store(unix - Instant::now().as_secs() as i64, Ordering::Release);
        Ok(())
    })
    .await
    .map_err(|_| Error::Clock)?
}
