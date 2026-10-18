use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use embedded_io_async::{ErrorKind, ErrorType, Read, Write};
use reqwless::{
    request::{Method, Request, RequestBuilder},
    response::Response,
};
use timetable_firmware::cooperative::Cooperative;
#[path = "../src/http_request.rs"]
mod http_request;
use http_request::{Stream, send_get};

// Like TLS: small writes stay buffered, and a read does not implicitly flush.
struct BufferedPeer {
    pending: Vec<u8>,
    sent: Vec<u8>,
    flushes: usize,
    reads: usize,
    fail_write: bool,
    fail_flush: bool,
}
impl BufferedPeer {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
            sent: Vec::new(),
            flushes: 0,
            reads: 0,
            fail_write: false,
            fail_flush: false,
        }
    }
}
impl ErrorType for BufferedPeer {
    type Error = ErrorKind;
}
impl Write for BufferedPeer {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize, ErrorKind> {
        if self.fail_write {
            return Err(ErrorKind::BrokenPipe);
        }
        let n = bytes.len().min(3); // Exercise write_all's partial-write contract.
        self.pending.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    async fn flush(&mut self) -> Result<(), ErrorKind> {
        self.flushes += 1;
        if self.fail_flush {
            return Err(ErrorKind::BrokenPipe);
        }
        self.sent.append(&mut self.pending);
        Ok(())
    }
}
impl Read for BufferedPeer {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, ErrorKind> {
        self.reads += 1;
        if !self.sent.ends_with(b"\r\n\r\n") {
            return Err(ErrorKind::TimedOut);
        }
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nzip";
        assert!(out.len() >= response.len());
        out[..response.len()].copy_from_slice(response);
        Ok(response.len())
    }
}
fn ready<T>(future: impl Future<Output = T>) -> T {
    match pin!(future)
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending fixture"),
    }
}
#[test]
fn get_flushes_the_tls_request_before_reading_response() {
    let mut peer = BufferedPeer::new();
    {
        let mut stream = Stream(Cooperative::new(&mut peer));
        ready(send_get(&mut stream, "/feed.zip", "www.ztm.poznan.pl")).unwrap();
        let mut headers = [0; 256];
        let response = ready(Response::read(&mut stream, Method::GET, &mut headers)).unwrap();
        assert_eq!(response.status.0, 200);
        let mut body = [0; 3];
        assert_eq!(ready(response.body().reader().read(&mut body)).unwrap(), 3);
        assert_eq!(&body, b"zip");
    }
    assert_eq!(peer.flushes, 1);
    assert_eq!(peer.reads, 1);
    assert!(peer.pending.is_empty());
    assert_eq!(peer.sent, b"GET /feed.zip HTTP/1.1\r\nHost: www.ztm.poznan.pl\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n");
}
#[test]
fn previous_header_only_path_times_out_with_buffered_tls() {
    let mut stream = Stream(Cooperative::new(BufferedPeer::new()));
    ready(
        Request::get("/feed.zip")
            .host("www.ztm.poznan.pl")
            .build()
            .write_header(&mut stream),
    )
    .unwrap();
    let mut headers = [0; 256];
    assert!(matches!(
        ready(Response::read(&mut stream, Method::GET, &mut headers)),
        Err(reqwless::Error::Network(ErrorKind::TimedOut))
    ));
}
#[test]
fn write_and_flush_errors_stop_the_request_without_response_reads() {
    for write in [true, false] {
        let mut peer = BufferedPeer::new();
        peer.fail_write = write;
        peer.fail_flush = !write;
        {
            let mut stream = Stream(Cooperative::new(&mut peer));
            assert!(matches!(
                ready(send_get(&mut stream, "/feed.zip", "www.ztm.poznan.pl")),
                Err(reqwless::Error::Network(ErrorKind::BrokenPipe))
            ));
        }
        assert!(peer.sent.is_empty());
        assert_eq!(peer.reads, 0);
        assert_eq!(peer.flushes, usize::from(!write));
    }
}
