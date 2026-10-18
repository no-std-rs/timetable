//! Bound ready-to-read streaming work between executor scheduling points.
use core::{future::poll_fn, task::Poll};
use embedded_io_async::{ErrorType, Read, Write};

const QUANTUM: usize = 2048;

pub struct Cooperative<R> {
    inner: R,
    remaining: usize,
}

impl<R> Cooperative<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            remaining: QUANTUM,
        }
    }
}

impl<R: ErrorType> ErrorType for Cooperative<R> {
    type Error = R::Error;
}

impl<R: Read> Read for Cooperative<R> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut yielded = false;
            poll_fn(|cx| {
                if yielded {
                    Poll::Ready(())
                } else {
                    yielded = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            })
            .await;
            self.remaining = QUANTUM;
        }
        let limit = out.len().min(self.remaining);
        let n = self.inner.read(&mut out[..limit]).await?;
        self.remaining -= n;
        Ok(n)
    }
}

impl<R: Write> Write for Cooperative<R> {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.inner.write(bytes).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush().await
    }
}
