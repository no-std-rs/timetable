//! Send a complete GET before waiting for its response on a buffered transport.
use embedded_io_async::{Error as _, ErrorType, Read, Write};
use reqwless::request::{Request, RequestBuilder};

// Only ClientHello/Finished and short GET headers are sent. Incoming server
// records keep their full 16 KiB receive allowance.
pub const TLS_TX_BYTES: usize = 1024;

pub struct Endpoint<'a> {
    pub host: &'static str,
    pub path: &'a str,
    pub root: &'static [u8],
    pub redundant_root: Option<&'static [u8]>,
}
pub fn endpoint(url: &str) -> Result<Endpoint<'_>, timetable_firmware::download::Error> {
    for (host, root, redundant_root) in [
        (
            "www.ztm.poznan.pl",
            &include_bytes!("../certs/certum-trusted-root.der")[..],
            None,
        ),
        (
            "mobilitydatabase.org",
            &include_bytes!("../certs/gts-root-r4.der")[..],
            Some(&include_bytes!("../certs/mobility-cross.der")[..]),
        ),
        (
            "files.mobilitydatabase.org",
            &include_bytes!("../certs/gts-root-r1.der")[..],
            Some(&include_bytes!("../certs/mobility-files-cross.der")[..]),
        ),
    ] {
        if let Some(path) = url
            .strip_prefix("https://")
            .and_then(|s| s.strip_prefix(host))
            .filter(|s| s.starts_with('/'))
        {
            if path.bytes().any(|b| b <= 32 || b == 127 || b == b'#') {
                break;
            }
            return Ok(Endpoint {
                host,
                path,
                root,
                redundant_root,
            });
        }
    }
    Err(timetable_firmware::download::Error::Http)
}

/// Adapt a caller-owned stream to reqwless without a second read buffer.
pub struct Stream<S>(pub S);
impl<S: ErrorType> ErrorType for Stream<S> {
    type Error = S::Error;
}
impl<S: Read> Read for Stream<S> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(out).await
    }
}
impl<S: Write> Write for Stream<S> {
    async fn write(&mut self, data: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(data).await
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}
impl<S: Read> reqwless::TryBufRead for Stream<S> {}

pub async fn send_get<S: Write>(
    stream: &mut S,
    path: &str,
    host: &str,
) -> Result<(), reqwless::Error> {
    send_get_range(stream, path, host, None).await
}

pub async fn send_get_range<S: Write>(
    stream: &mut S,
    path: &str,
    host: &str,
    range: Option<&str>,
) -> Result<(), reqwless::Error> {
    send_get_connection(stream, path, host, range, false).await
}

pub async fn send_get_connection<S: Write>(
    stream: &mut S,
    path: &str,
    host: &str,
    range: Option<&str>,
    persistent: bool,
) -> Result<(), reqwless::Error> {
    send_get_conditional(stream, path, host, range, persistent, None).await
}

pub async fn send_get_conditional<S: Write>(
    stream: &mut S,
    path: &str,
    host: &str,
    range: Option<&str>,
    persistent: bool,
    validator: Option<&timetable_firmware::download::Validator>,
) -> Result<(), reqwless::Error> {
    let conditional = validator.and_then(|v| v.header());
    let mut headers = [
        ("Accept-Encoding", "identity"),
        (
            "Connection",
            if persistent { "keep-alive" } else { "close" },
        ),
        ("User-Agent", "timetable/0.1"),
        ("Range", ""),
        ("If-None-Match", ""),
    ];
    let mut count = 3;
    if let Some(range) = range {
        headers[count] = ("Range", range);
        count += 1;
    }
    if let Some(header) = conditional {
        headers[count] = header;
        count += 1;
    }
    Request::get(path)
        .host(host)
        .headers(&headers[..count])
        .build()
        .write_header(stream)
        .await?;
    stream
        .flush()
        .await
        .map_err(|e| reqwless::Error::Network(e.kind()))
}

pub fn validate<'a>(
    status: u16,
    headers: impl Iterator<Item = (&'a str, &'a [u8])>,
    range: Option<(gtfs::http::ByteRange, Option<u64>)>,
) -> Result<timetable_firmware::download::Metadata, timetable_firmware::download::Error> {
    use timetable_firmware::download::{Error, Metadata};
    if status != if range.is_some() { 206 } else { 200 } {
        return Err(Error::Http);
    }
    let mut content_range = None;
    let mut length = None;
    let mut encoding = false;
    let mut etag = None;
    let mut modified = None;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("etag") && etag.replace(value).is_some()
            || name.eq_ignore_ascii_case("last-modified") && modified.replace(value).is_some()
        {
            return Err(Error::Http);
        }
        if name.eq_ignore_ascii_case("content-encoding") {
            if encoding || value != b"identity" {
                return Err(Error::Http);
            }
            encoding = true;
        }
        if name.eq_ignore_ascii_case("content-range") && content_range.replace(value).is_some() {
            return Err(Error::Http);
        }
        if name.eq_ignore_ascii_case("content-length") {
            if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
                return Err(Error::Http);
            }
            let n = core::str::from_utf8(value)
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or(Error::Http)?;
            if length.replace(n).is_some() {
                return Err(Error::Http);
            }
        }
    }
    let mut validator = etag
        .map(|v| timetable_firmware::download::Validator::new("etag", v))
        .unwrap_or_default();
    if validator.header().is_none() {
        validator = modified
            .map(|v| timetable_firmware::download::Validator::new("last-modified", v))
            .unwrap_or_default();
    }
    if let Some((range, expected)) = range {
        let total = range.validate_content_range(content_range.ok_or(Error::Http)?)?;
        if expected.is_some_and(|n| n != total) || length.is_some_and(|n| n != range.len()) {
            return Err(Error::Http);
        }
        Ok(Metadata {
            length: Some(total),
            modified: None,
            validator,
        })
    } else {
        if content_range.is_some() {
            return Err(Error::Http);
        }
        Ok(Metadata {
            length,
            modified: None,
            validator,
        })
    }
}
