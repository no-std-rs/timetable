use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use embedded_io_async::{ErrorKind, ErrorType, Read, Write};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use timetable_firmware::cooperative::Cooperative;

#[derive(Default)]
struct Wakes(AtomicUsize);
impl std::task::Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn always_ready_download_yields_and_wakes_without_losing_bytes() {
    let source = [0x5a; 5000];
    let mut reader = Cooperative::new(source.as_slice());
    let wake = Arc::new(Wakes::default());
    let waker = Waker::from(wake.clone());
    let mut cx = Context::from_waker(&waker);
    let mut out = [0; 4096];
    let mut output = Vec::new();
    {
        let mut first = pin!(reader.read(&mut out));
        assert_eq!(first.as_mut().poll(&mut cx), Poll::Ready(Ok(2048)));
    }
    output.extend_from_slice(&out[..2048]);
    {
        let mut empty = pin!(reader.read(&mut []));
        assert_eq!(empty.as_mut().poll(&mut cx), Poll::Ready(Ok(0)));
    }
    for count in [2048, 904] {
        {
            let mut next = pin!(reader.read(&mut out));
            assert!(next.as_mut().poll(&mut cx).is_pending());
            assert_eq!(next.as_mut().poll(&mut cx), Poll::Ready(Ok(count)));
        }
        output.extend_from_slice(&out[..count]);
    }
    assert_eq!(wake.0.load(Ordering::Relaxed), 2);
    assert_eq!(output, source);
    let mut eof = pin!(reader.read(&mut out));
    assert_eq!(eof.as_mut().poll(&mut cx), Poll::Ready(Ok(0)));
}

struct ErrorIo {
    writes: usize,
    flushes: usize,
}
impl ErrorType for ErrorIo {
    type Error = ErrorKind;
}
impl Read for ErrorIo {
    async fn read(&mut self, _: &mut [u8]) -> Result<usize, ErrorKind> {
        Err(ErrorKind::Other)
    }
}
impl Write for ErrorIo {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize, ErrorKind> {
        self.writes += 1;
        Ok(bytes.len().min(3))
    }
    async fn flush(&mut self) -> Result<(), ErrorKind> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn partial_reads_and_io_errors_keep_the_underlying_contract() {
    let mut io = Cooperative::new(&b"1234"[..]);
    let mut cx = Context::from_waker(Waker::noop());
    let mut out = [0; 2];
    for expected in [*b"12", *b"34"] {
        {
            let mut read = pin!(io.read(&mut out));
            assert_eq!(read.as_mut().poll(&mut cx), Poll::Ready(Ok(2)));
        }
        assert_eq!(out, expected);
    }
    let mut inner = ErrorIo {
        writes: 0,
        flushes: 0,
    };
    {
        let mut io = Cooperative::new(&mut inner);
        assert_eq!(
            pin!(io.read(&mut out)).as_mut().poll(&mut cx),
            Poll::Ready(Err(ErrorKind::Other))
        );
        assert_eq!(
            pin!(io.write(b"abcdef")).as_mut().poll(&mut cx),
            Poll::Ready(Ok(3))
        );
        assert_eq!(pin!(io.flush()).as_mut().poll(&mut cx), Poll::Ready(Ok(())));
    }
    assert_eq!((inner.writes, inner.flushes), (1, 1));
}
