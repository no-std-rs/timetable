// Root pins and TLS storage are exercised by the transport/verifier tests.
#[allow(dead_code)]
#[path = "../src/http_request.rs"]
mod http_request;
#[path = "../src/http_session.rs"]
mod http_session;
use core::{
    future::Future,
    task::{Context, Poll, Waker},
};
use embedded_io_async::{ErrorType, Read, Write};
use std::{cell::RefCell, rc::Rc};
use timetable_firmware::download::{Error, Fill, Http, Operation};

#[derive(Default)]
struct State {
    responses: Vec<Vec<u8>>,
    sent: Vec<u8>,
    current: usize,
    offset: usize,
    flushes: usize,
    pending: bool,
}
struct Peer(Rc<RefCell<State>>);
impl ErrorType for Peer {
    type Error = embedded_io_async::ErrorKind;
}
impl Write for Peer {
    async fn write(&mut self, data: &[u8]) -> Result<usize, Self::Error> {
        let n = data.len().min(5);
        self.0.borrow_mut().sent.extend_from_slice(&data[..n]);
        Ok(n)
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        let mut s = self.0.borrow_mut();
        s.current = s.flushes;
        s.offset = 0;
        s.flushes += 1;
        Ok(())
    }
}
impl Read for Peer {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        if self.0.borrow().pending {
            core::future::pending::<()>().await;
        }
        let mut s = self.0.borrow_mut();
        assert!(s.flushes > 0, "GET must be flushed before reading");
        let body = &s.responses[s.current];
        let n = out.len().min(7).min(body.len().saturating_sub(s.offset));
        out[..n].copy_from_slice(&body[s.offset..s.offset + n]);
        s.offset += n;
        Ok(n)
    }
}
struct Observer;
impl http_session::Observe for Observer {
    fn millis(&self) -> u64 {
        0
    }
    fn now(&self) -> Option<i64> {
        Some(1792367882)
    }
}
fn run<T>(future: impl Future<Output = T>) -> T {
    match core::pin::pin!(future)
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("unexpected pending fixture"),
    }
}
const URL: &str = "https://files.mobilitydatabase.org/test.zip";
fn setup(
    responses: &[&[u8]],
) -> (
    http_session::Connection<'static, Peer, Observer>,
    Rc<RefCell<State>>,
) {
    let state = Rc::new(RefCell::new(State {
        responses: responses.iter().map(|b| b.to_vec()).collect(),
        ..State::default()
    }));
    (
        http_session::Connection::new(
            Peer(state.clone()),
            "files.mobilitydatabase.org",
            &Observer,
            true,
        ),
        state,
    )
}
#[test]
fn sequential_responses_share_one_stream_and_validate_each_body_before_reuse() {
    let (client, peer) = setup(&[
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\none",
        b"HTTP/1.1 206 OK\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/10\r\n\r\ntwo",
    ]);
    let mut bytes = [0; 3];
    run(client.fetch(URL, None, &mut Fill(&mut bytes))).unwrap();
    assert_eq!(&bytes, b"one");
    run(client.fetch(
        URL,
        Some((gtfs::http::ByteRange::new(3, 5).unwrap(), Some(10))),
        &mut Fill(&mut bytes),
    ))
    .unwrap();
    assert_eq!(&bytes, b"two");
    let state = peer.borrow();
    assert_eq!(state.flushes, 2);
    let sent = std::str::from_utf8(&state.sent).unwrap();
    assert_eq!(sent.matches("Connection: keep-alive").count(), 2);
}
struct Fail;
impl Operation for Fail {
    async fn read<R: Read>(&mut self, mut body: R) -> Result<(), Error> {
        let mut b = [0];
        body.read(&mut b).await.map_err(|_| Error::Network)?;
        Err(Error::Feed(gtfs::Error::CrcMismatch))
    }
}
struct Early;
impl Operation for Early {
    async fn read<R: Read>(&mut self, _: R) -> Result<(), Error> {
        Ok(())
    }
}
#[test]
fn error_cancel_early_return_and_truncation_never_reuse_a_dirty_stream() {
    for kind in 0..4 {
        let (client, peer) = setup(&[b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\none"]);
        let result = match kind {
            0 => run(client.fetch(URL, None, &mut Fail)),
            1 => run(client.fetch(URL, None, &mut Early)),
            2 => {
                peer.borrow_mut().responses[0].pop();
                run(client.fetch(URL, None, &mut Fill(&mut [0; 3])))
            }
            _ => {
                peer.borrow_mut().pending = true;
                let mut bytes = [0; 3];
                let mut fill = Fill(&mut bytes);
                let mut f = core::pin::pin!(client.fetch(URL, None, &mut fill));
                assert!(
                    f.as_mut()
                        .poll(&mut Context::from_waker(Waker::noop()))
                        .is_pending()
                );
                Err(Error::Network)
            }
        };
        assert!(result.is_err());
        assert!(run(client.fetch(URL, None, &mut Fill(&mut [0; 3]))).is_err());
        assert_eq!(peer.borrow().flushes, 1);
    }
}
#[test]
fn close_delimited_or_explicit_close_cannot_be_reused_and_origin_cannot_change() {
    for response in [
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: keep-alive, CLOSE\r\n\r\none"
            .as_slice(),
        b"HTTP/1.1 200 OK\r\n\r\none",
    ] {
        let (client, peer) = setup(&[response]);
        run(client.fetch(URL, None, &mut Fill(&mut [0; 3]))).unwrap();
        assert!(run(client.fetch(URL, None, &mut Fill(&mut [0; 3]))).is_err());
        assert_eq!(peer.borrow().flushes, 1);
    }
    let (client, peer) = setup(&[]);
    assert!(run(client.fetch("https://mobilitydatabase.org/", None, &mut Fill(&mut []))).is_err());
    assert_eq!(peer.borrow().flushes, 0);
}

#[test]
fn conditional_304_has_no_body_even_with_content_length_and_connection_is_reusable() {
    let validator = timetable_firmware::download::Validator::new("etag", b"\"old\"");
    let (client, peer) = setup(&[
        b"HTTP/1.1 304 Not Modified\r\nContent-Length: 9000000\r\nETag: W/\"old\"\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\none",
    ]);
    assert!(run(client.unchanged(URL, &validator)).unwrap());
    let mut bytes = [0; 3];
    run(client.fetch(URL, None, &mut Fill(&mut bytes))).unwrap();
    assert_eq!(&bytes, b"one");
    let peer = peer.borrow();
    let sent = std::str::from_utf8(&peer.sent).unwrap();
    assert!(sent.contains("If-None-Match: \"old\""));
    assert!(sent.contains("Range: bytes=0-0"));
}
#[test]
fn unsolicited_or_mismatched_304_is_rejected_without_reusing_the_connection() {
    let validator = timetable_firmware::download::Validator::new("etag", b"\"old\"");
    for response in [
        b"HTTP/1.1 304 Not Modified\r\nETag: \"new\"\r\n\r\n".as_slice(),
        b"HTTP/1.1 304 Not Modified\r\nETag: \"old\"\r\nETag: \"old\"\r\n\r\n",
    ] {
        let (client, peer) = setup(&[response]);
        assert!(run(client.unchanged(URL, &validator)).is_err());
        assert!(run(client.fetch(URL, None, &mut Fill(&mut [0]))).is_err());
        assert_eq!(peer.borrow().flushes, 1);
    }
    let (client, _) = setup(&[b"HTTP/1.1 304 Not Modified\r\n\r\n"]);
    assert!(run(client.fetch(URL, None, &mut Fill(&mut [0]))).is_err());
    let bad = timetable_firmware::download::Validator::new("etag", b"\"bad\"\r\nInjected: yes");
    assert!(bad.header().is_none());
    let date = timetable_firmware::download::Validator::new(
        "last-modified",
        b"Wed, 21 Oct 2015 07:28:00 GMT",
    );
    assert_eq!(
        date.header(),
        Some(("If-Modified-Since", "Wed, 21 Oct 2015 07:28:00 GMT"))
    );
}

#[test]
fn range_identity_never_uses_weak_etags_or_last_modified_dates() {
    use timetable_firmware::download::Validator;
    let strong = Validator::new("etag", b"\"same\"");
    let weak = Validator::new("etag", b"W/\"same\"");
    let date = Validator::new("last-modified", b"Wed, 21 Oct 2015 07:28:00 GMT");
    assert!(strong.verify(&strong).is_ok());
    for (expected, actual) in [(strong, weak), (weak, strong), (weak, weak), (date, date)] {
        assert!(expected.verify(&actual).is_err());
    }
    assert!(weak.for_ranges().header().is_none());
    assert!(date.for_ranges().header().is_none());
    let empty = Validator::default();
    assert!(empty.verify(&empty).is_ok());
}
