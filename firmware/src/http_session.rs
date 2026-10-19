//! Sequential HTTP/1.1 requests on one borrowed, verified TLS stream.
use core::cell::Cell;
use embedded_io_async::{ErrorType, Read, Write};
use gtfs::http::ByteRange;
use reqwless::{request::Method, response::Response};
use timetable_firmware::download::{Error, Http, Metadata, Operation};

pub trait Observe {
    fn millis(&self) -> u64;
    fn now(&self) -> Option<i64>;
    fn stage(&self, _message: &str) {}
    fn request(&self, _url: &str, _range: Option<(ByteRange, Option<u64>)>) {}
    fn status(&self, _status: u16) {}
    fn received(&self, _bytes: usize) {}
    fn progress(&self, _bytes: u64, _total: Option<u64>, _elapsed_ms: u64) {}
    fn complete(&self, _bytes: u64, _elapsed_ms: u64) {}
}
pub struct Connection<'a, S, O> {
    stream: Cell<Option<S>>,
    host: &'static str,
    observe: &'a O,
    persistent: bool,
}
impl<'a, S, O> Connection<'a, S, O> {
    pub fn new(stream: S, host: &'static str, observe: &'a O, persistent: bool) -> Self {
        Self {
            stream: Cell::new(Some(stream)),
            host,
            observe,
            persistent,
        }
    }
}
struct Body<'a, R, O> {
    inner: R,
    observe: &'a O,
    bytes: u64,
    expected: Option<u64>,
    started: u64,
    report: u64,
}
impl<R: ErrorType, O> ErrorType for Body<'_, R, O> {
    type Error = R::Error;
}
impl<R: Read, O: Observe> Read for Body<'_, R, O> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let n = self.inner.read(out).await?;
        self.bytes += n as u64;
        self.observe.received(n);
        let now = self.observe.millis();
        if now.saturating_sub(self.report) >= 5000 {
            self.observe
                .progress(self.bytes, self.expected, now.saturating_sub(self.started));
            self.report = now;
        }
        Ok(n)
    }
}
impl<S: Read + Write, O: Observe> Http for Connection<'_, S, O> {
    fn stage(&self, message: &str) {
        self.observe.stage(message);
    }
    async fn fetch(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        operation: &mut impl Operation,
    ) -> Result<Metadata, Error> {
        let endpoint = crate::http_request::endpoint(url)?;
        if endpoint.host != self.host {
            return Err(Error::Http);
        }
        // A failure/cancellation poisons this connection before any I/O. Its
        // owning session is then dropped; provisional callbacks are not retried.
        let mut stream = self.stream.take().ok_or(Error::Http)?;
        self.observe.request(url, range);
        let mut text = [0; 64];
        let header = range.map(|(r, _)| r.header(&mut text)).transpose()?;
        let sent = if self.persistent {
            crate::http_request::send_get_connection(
                &mut stream,
                endpoint.path,
                self.host,
                header,
                true,
            )
            .await
        } else if header.is_some() {
            crate::http_request::send_get_range(&mut stream, endpoint.path, self.host, header).await
        } else {
            crate::http_request::send_get(&mut stream, endpoint.path, self.host).await
        };
        sent.map_err(|_| Error::Http)?;
        let mut headers = [0; 2048];
        let response = Response::read(&mut stream, Method::GET, &mut headers)
            .await
            .map_err(|_| Error::Http)?;
        self.observe.status(response.status.0);
        let mut meta = crate::http_request::validate(response.status.0, response.headers(), range)?;
        meta.modified = self
            .observe
            .now()
            .and_then(|now| timetable_display::freshness::source_modified(response.headers(), now));
        // Reuse requires an unambiguous self-delimited response. Close-delimited
        // and chunked single requests still work, but are never reused.
        let mut length = None;
        let mut reusable = self.persistent;
        for (name, value) in response.headers() {
            if name.eq_ignore_ascii_case("content-length") {
                length = core::str::from_utf8(value)
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok());
            }
            if name.eq_ignore_ascii_case("transfer-encoding") {
                reusable = false;
            }
            if name.eq_ignore_ascii_case("connection")
                && value
                    .split(|b| *b == b',')
                    .any(|token| token.trim_ascii().eq_ignore_ascii_case(b"close"))
            {
                reusable = false;
            }
        }
        reusable &= length.is_some();
        let expected = range.map(|(r, _)| r.len()).or(length);
        let started = self.observe.millis();
        let mut body = Body {
            inner: response.body().reader(),
            observe: self.observe,
            bytes: 0,
            expected,
            started,
            report: started,
        };
        operation.read(&mut body).await?;
        if body.read(&mut [0]).await.map_err(|_| Error::Network)? != 0
            || expected.is_some_and(|n| body.bytes != n)
        {
            return Err(Error::Feed(gtfs::Error::SizeMismatch));
        }
        self.observe
            .complete(body.bytes, self.observe.millis().saturating_sub(started));
        if reusable {
            self.stream.set(Some(stream));
        }
        Ok(meta)
    }
}
