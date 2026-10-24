//! Sequential HTTP/1.1 requests on one borrowed, verified TLS stream.
use core::cell::Cell;
use embedded_io_async::{ErrorType, Read, Write};
use gtfs::http::ByteRange;
use reqwless::{request::Method, response::Response};
use timetable_firmware::download::{Error, Http, Metadata, Operation};

pub trait Observe {
    /// Acquired only inside a verified TLS session; the platform owns storage.
    fn journey_scratch(
        &self,
        _today: gtfs::Date,
    ) -> Result<impl AsMut<[timetable_display::journey::DevicePlanner]>, gtfs::Error> {
        Err::<[timetable_display::journey::DevicePlanner; 0], _>(gtfs::Error::BufferTooSmall)
    }
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
    async fn journey_plan(
        &self,
        builder: &mut timetable_display::feed::Builder,
        source: &mut impl timetable_display::feed::Tables,
        mask: u8,
        config: &timetable_display::Config,
        workspace: &mut gtfs::zip::Workspace,
    ) -> Result<(), gtfs::Error> {
        if !config.groups.iter().any(|g| g.journey.is_some()) {
            return Ok(());
        }
        let mut storage = self.observe.journey_scratch(builder.dates[1])?;
        let planner = storage
            .as_mut()
            .first_mut()
            .ok_or(gtfs::Error::BufferTooSmall)?;
        builder
            .device_journeys_with(planner, source, mask, config, workspace)
            .await
    }
    fn stage(&self, message: &str) {
        self.observe.stage(message);
    }
    async fn fetch(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        operation: &mut impl Operation,
    ) -> Result<Metadata, Error> {
        self.fetch_inner(url, range, operation, None)
            .await
            .map(|(meta, _)| meta)
    }
    async fn unchanged(
        &self,
        url: &str,
        validator: &timetable_firmware::download::Validator,
    ) -> Result<bool, Error> {
        if validator.header().is_none() {
            return Ok(false);
        }
        self.fetch_inner(
            url,
            Some((ByteRange::new(0, 0)?, None)),
            &mut timetable_firmware::download::Fill(&mut [0]),
            Some(validator),
        )
        .await
        .map(|(_, unchanged)| unchanged)
    }
}
impl<S: Read + Write, O: Observe> Connection<'_, S, O> {
    async fn fetch_inner(
        &self,
        url: &str,
        range: Option<(ByteRange, Option<u64>)>,
        operation: &mut impl Operation,
        validator: Option<&timetable_firmware::download::Validator>,
    ) -> Result<(Metadata, bool), Error> {
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
        let sent = if validator.is_some() {
            crate::http_request::send_get_conditional(
                &mut stream,
                endpoint.path,
                self.host,
                header,
                self.persistent,
                validator,
            )
            .await
        } else if self.persistent {
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
        if response.status.0 == 304 {
            let expected = validator.and_then(|v| v.header()).ok_or(Error::Http)?;
            let mut seen = false;
            let mut reusable = self.persistent;
            for (name, value) in response.headers() {
                if name.eq_ignore_ascii_case("etag") {
                    if seen {
                        return Err(Error::Http);
                    }
                    seen = true;
                    if expected.0 == "If-None-Match"
                        && value.strip_prefix(b"W/").unwrap_or(value)
                            != expected
                                .1
                                .as_bytes()
                                .strip_prefix(b"W/")
                                .unwrap_or(expected.1.as_bytes())
                    {
                        return Err(Error::Http);
                    }
                }
                if name.eq_ignore_ascii_case("connection")
                    && value
                        .split(|b| *b == b',')
                        .any(|v| v.trim_ascii().eq_ignore_ascii_case(b"close"))
                {
                    reusable = false;
                }
                if name.eq_ignore_ascii_case("transfer-encoding") {
                    return Err(Error::Http);
                }
            }
            // RFC 9110: a 304 ends after its headers. Content-Length describes
            // the selected representation, never a 304 response body.
            drop(response);
            if reusable {
                self.stream.set(Some(stream));
            }
            self.observe.complete(0, 0);
            return Ok((Metadata::default(), true));
        }
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
        Ok((meta, false))
    }
}
