//! Send a complete GET before waiting for its response on a buffered transport.
use embedded_io_async::{Error as _, ErrorType, Read, Write};
use reqwless::request::{Request, RequestBuilder};

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
    Request::get(path)
        .host(host)
        .headers(&[("Accept-Encoding", "identity"), ("Connection", "close")])
        .build()
        .write_header(stream)
        .await?;
    // embedded-tls buffers a short request until flush. Reading does not send
    // that buffer, so the peer otherwise waits for a request we never deliver.
    stream
        .flush()
        .await
        .map_err(|e| reqwless::Error::Network(e.kind()))
}
