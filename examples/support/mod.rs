//! Host-only TLS and file adapters. All ZIP, CSV and query work is in the
//! allocation-free library. reqwless handles HTTP, including chunked bodies.

use embedded_io_adapters::tokio_1::FromTokio;
use embedded_io_async::{ErrorType, Read, Seek, SeekFrom, Write};
use reqwless::{
    TryBufRead,
    request::{Method, Request, RequestBuilder},
    response::Response,
};
use std::{fs::File, io, sync::Arc};
use timetable::{
    Error, Table, TripIndex, csv,
    http::{ByteRange, RangeFetch, RangeReader},
    stream, zip,
};
use tokio::net::TcpStream;
use tokio_rustls::{
    TlsConnector,
    client::TlsStream,
    rustls::{ClientConfig, RootCertStore, pki_types::ServerName},
};

pub type AnyError = Box<dyn std::error::Error>;

pub struct Counting<'a, R> {
    pub source: R,
    pub bytes: &'a mut u64,
}
impl<R: ErrorType> ErrorType for Counting<'_, R> {
    type Error = R::Error;
}
impl<R: Read> Read for Counting<'_, R> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let n = self.source.read(out).await?;
        *self.bytes += n as u64;
        Ok(n)
    }
}

pub struct Stream(FromTokio<TlsStream<TcpStream>>);
impl ErrorType for Stream {
    type Error = io::Error;
}
impl Read for Stream {
    async fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.0.read(b).await
    }
}
impl Write for Stream {
    async fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.write(b).await
    }
    async fn flush(&mut self) -> io::Result<()> {
        self.0.flush().await
    }
}
impl TryBufRead for Stream {}

pub struct FileIo(pub File);
impl ErrorType for FileIo {
    type Error = io::Error;
}
impl Read for FileIo {
    async fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.0, b)
    }
}
impl Seek for FileIo {
    async fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        io::Seek::seek(&mut self.0, p.into())
    }
}

pub fn endpoint(url: &str) -> Result<(&str, &str), AnyError> {
    let rest = url
        .strip_prefix("https://")
        .ok_or("expected an HTTPS URL")?;
    let (host, path) = rest
        .split_once('/')
        .map_or((rest, "/"), |(host, _)| (host, &rest[host.len()..]));
    if host.is_empty() || host.contains([':', '@']) || url.bytes().any(|b| b <= 32 || b == 127) {
        return Err("invalid HTTPS URL".into());
    }
    Ok((host, path))
}

/// This generic request function also works with an embedded TLS socket.
pub async fn get<'a, S: Read + Write + TryBufRead>(
    stream: &'a mut S,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    rx: &'a mut [u8],
) -> Result<Response<'a, 'a, S>, reqwless::Error> {
    Request::get(path)
        .host(host)
        .headers(headers)
        .build()
        .write_header(stream)
        .await?;
    stream
        .flush()
        .await
        .map_err(|_| reqwless::Error::ConnectionAborted)?;
    Response::read(stream, Method::GET, rx).await
}

pub struct Http {
    tls: Arc<ClientConfig>,
    pub bytes: u64,
    pub requests: usize,
}
impl Http {
    pub fn new() -> Self {
        let roots = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let tls = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Self {
            tls: Arc::new(tls),
            bytes: 0,
            requests: 0,
        }
    }
    pub async fn connect(&mut self, url: &str) -> Result<Stream, AnyError> {
        let (host, _) = endpoint(url)?;
        let tcp = TcpStream::connect((host, 443)).await?;
        tcp.set_nodelay(true)?;
        let name = ServerName::try_from(host.to_owned())?;
        let tls = TlsConnector::from(self.tls.clone())
            .connect(name, tcp)
            .await?;
        self.requests += 1;
        Ok(Stream(FromTokio::new(tls)))
    }

    pub async fn probe(&mut self, url: &str) -> Result<u64, AnyError> {
        let mut stream = self.connect(url).await?;
        let (host, path) = endpoint(url)?;
        let mut rx = [0; 4096];
        let response = get(
            &mut stream,
            host,
            path,
            &[
                ("Range", "bytes=0-0"),
                ("Accept-Encoding", "identity"),
                ("Connection", "close"),
            ],
            &mut rx,
        )
        .await?;
        let length = validate(&response, ByteRange::new(0, 0)?, None)?;
        let mut body = response.body().reader();
        let mut b = [0; 1];
        body.read_exact(&mut b)
            .await
            .map_err(|_| Error::UnexpectedEof)?;
        if body.read(&mut b).await? != 0 {
            return Err(Error::SizeMismatch.into());
        }
        self.bytes += 1;
        Ok(length)
    }

    pub async fn directory(&mut self, url: &str, length: u64) -> Result<zip::Directory, AnyError> {
        let mut cache = [0; 2048];
        let fetcher = RangeHttp {
            http: self,
            url,
            length,
        };
        let mut source = RangeReader::new(fetcher, length, &mut cache)?;
        Ok(zip::Directory::read(&mut source, &mut [0; 1024]).await?)
    }

    /// Exactly two requests per table: local header, then the compressed payload.
    pub async fn scan(
        &mut self,
        url: &str,
        length: u64,
        member: zip::Member,
        workspace: &mut zip::Workspace,
        mut visit: impl FnMut(csv::Record<'_>) -> Result<(), Error>,
    ) -> Result<(), AnyError> {
        let mut header = [0; 30];
        RangeHttp {
            http: self,
            url,
            length,
        }
        .fetch(member.header_range(), &mut header)
        .await?;
        let range = member.data_range(&header)?;
        let mut value = [0; 64];
        let value = range.header(&mut value)?;
        let mut stream = self.connect(url).await?;
        let (host, path) = endpoint(url)?;
        let mut rx = [0; 4096];
        let response = get(
            &mut stream,
            host,
            path,
            &[
                ("Range", value),
                ("Accept-Encoding", "identity"),
                ("Connection", "close"),
            ],
            &mut rx,
        )
        .await?;
        validate(&response, range, Some(length))?;
        let source = zip::Reader::data(response.body().reader(), workspace, member);
        let mut csv = csv::Reader::<_>::new(source, member.table());
        while let Some(row) = csv.next().await? {
            visit(row)?;
        }
        self.bytes += range.len();
        Ok(())
    }

    pub async fn download(&mut self, url: &str, path: &str) -> Result<(), AnyError> {
        let mut stream = self.connect(url).await?;
        let (host, request_path) = endpoint(url)?;
        let mut rx = [0; 4096];
        let response = get(
            &mut stream,
            host,
            request_path,
            &[("Accept-Encoding", "identity"), ("Connection", "close")],
            &mut rx,
        )
        .await?;
        if response.status.0 != 200 {
            return Err(format!(
                "HTTP {} (use a direct URL; redirects are not followed)",
                response.status.0
            )
            .into());
        }
        let mut body = response.body().reader();
        let temporary = format!("{path}.part");
        let mut file = File::create(&temporary)?;
        let mut buf = [0; 8192];
        loop {
            let n = body.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            io::Write::write_all(&mut file, &buf[..n])?;
            self.bytes += n as u64;
        }
        file.sync_all()?;
        drop(file);
        // Verify every member before replacing the previous cached snapshot.
        let source = FileIo(File::open(&temporary)?);
        zip::verify(source, &mut zip::Workspace::new(), &mut buf).await?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }

    pub async fn scan_original(
        &mut self,
        url: &str,
        tables: &[Table],
        workspace: &mut zip::Workspace,
        visit: impl FnMut(csv::Record<'_>) -> Result<(), Error>,
    ) -> Result<zip::Manifest, AnyError> {
        let mut socket = self.connect(url).await?;
        let (host, path) = endpoint(url)?;
        let mut rx = [0; 4096];
        let response = get(
            &mut socket,
            host,
            path,
            &[("Accept-Encoding", "identity"), ("Connection", "close")],
            &mut rx,
        )
        .await?;
        full_response(&response)?;
        let body = Counting {
            source: response.body().reader(),
            bytes: &mut self.bytes,
        };
        Ok(stream::scan(body, tables, None, workspace, visit).await?)
    }

    pub async fn prepare_original<const N: usize, const S: usize>(
        &mut self,
        url: &str,
        selection: stream::Selection<'_>,
        trips: &mut TripIndex<'_, N>,
        services: &mut stream::Services<'_, S>,
        workspace: &mut zip::Workspace,
    ) -> Result<zip::Manifest, AnyError> {
        let mut socket = self.connect(url).await?;
        let (host, path) = endpoint(url)?;
        let mut rx = [0; 4096];
        let response = get(
            &mut socket,
            host,
            path,
            &[("Accept-Encoding", "identity"), ("Connection", "close")],
            &mut rx,
        )
        .await?;
        full_response(&response)?;
        let body = Counting {
            source: response.body().reader(),
            bytes: &mut self.bytes,
        };
        Ok(stream::prepare(body, selection, trips, services, workspace).await?)
    }

    pub async fn collect_original<const N: usize, const O: usize>(
        &mut self,
        url: &str,
        manifest: &zip::Manifest,
        trips: &TripIndex<'_, N>,
        stops: &[&str],
        workspace: &mut zip::Workspace,
        output: &mut timetable::query::DepartureBuffer<'_, O>,
    ) -> Result<(), AnyError> {
        output.clear();
        let mut socket = self.connect(url).await?;
        let (host, path) = endpoint(url)?;
        let mut rx = [0; 4096];
        let response = get(
            &mut socket,
            host,
            path,
            &[("Accept-Encoding", "identity"), ("Connection", "close")],
            &mut rx,
        )
        .await?;
        full_response(&response)?;
        let body = Counting {
            source: response.body().reader(),
            bytes: &mut self.bytes,
        };
        Ok(stream::collect(body, manifest, trips, stops, workspace, output).await?)
    }
}

fn full_response<S: Read>(response: &Response<'_, '_, S>) -> Result<(), AnyError> {
    if response.status.0 != 200 {
        return Err(format!("HTTP {}", response.status.0).into());
    }
    for (name, value) in response.headers() {
        if name.eq_ignore_ascii_case("content-encoding") && value != b"identity" {
            return Err(Error::InvalidZip.into());
        }
    }
    Ok(())
}

fn validate<S: Read>(
    response: &Response<'_, '_, S>,
    range: ByteRange,
    length: Option<u64>,
) -> Result<u64, Error> {
    if response.status.0 != 206 {
        return Err(Error::RangeNotSupported);
    }
    let mut total = None;
    for (name, value) in response.headers() {
        if name.eq_ignore_ascii_case("content-range") {
            total = Some(range.validate_content_range(value)?);
        }
        if name.eq_ignore_ascii_case("content-encoding") && value != b"identity" {
            return Err(Error::InvalidRange);
        }
    }
    let total = total.ok_or(Error::InvalidRange)?;
    if length.is_some_and(|n| n != total)
        || response
            .content_length
            .is_some_and(|n| n as u64 != range.len())
    {
        return Err(Error::InvalidRange);
    }
    Ok(total)
}

pub struct RangeHttp<'a> {
    http: &'a mut Http,
    url: &'a str,
    length: u64,
}
impl RangeFetch for RangeHttp<'_> {
    async fn fetch(&mut self, range: ByteRange, out: &mut [u8]) -> Result<(), Error> {
        if out.len() as u64 != range.len() {
            return Err(Error::InvalidRange);
        }
        let mut stream = self
            .http
            .connect(self.url)
            .await
            .map_err(|_| Error::Io(embedded_io_async::ErrorKind::ConnectionAborted))?;
        let (host, path) = endpoint(self.url).map_err(|_| Error::InvalidRange)?;
        let mut value = [0; 64];
        let value = range.header(&mut value)?;
        let mut rx = [0; 4096];
        let response = get(
            &mut stream,
            host,
            path,
            &[
                ("Range", value),
                ("Accept-Encoding", "identity"),
                ("Connection", "close"),
            ],
            &mut rx,
        )
        .await
        .map_err(|_| Error::Io(embedded_io_async::ErrorKind::ConnectionAborted))?;
        validate(&response, range, Some(self.length))?;
        let mut body = response.body().reader();
        body.read_exact(out)
            .await
            .map_err(|_| Error::UnexpectedEof)?;
        if body
            .read(&mut [0; 1])
            .await
            .map_err(|_| Error::UnexpectedEof)?
            != 0
        {
            return Err(Error::SizeMismatch);
        }
        self.http.bytes += range.len();
        Ok(())
    }
}

pub fn member(directory: &zip::Directory, table: Table) -> Result<zip::Member, Error> {
    directory.member(table).ok_or(Error::MissingTable(table))
}
