//! Admission check around the unchanged TLS certificate verifier.
use embedded_tls::{CertificateRef, CertificateVerifyRef, TlsCipherSuite, TlsError, TlsVerifier};

// The pinned verifier's actual ZTM chain fits in a 24 KiB first-fit arena in
// the offline regression. This is admission headroom, not a radio peak bound:
// other radio tasks can still allocate, so their queues are bounded separately.
pub const VERIFY_HEAP_BYTES: usize = 24 * 1024;

pub struct HeapCheckedVerifier<V> {
    inner: V,
    available: fn() -> usize,
}
impl<V> HeapCheckedVerifier<V> {
    pub fn new(inner: V, available: fn() -> usize) -> Self {
        Self { inner, available }
    }
    fn check(&self) -> Result<(), TlsError> {
        if (self.available)() < VERIFY_HEAP_BYTES {
            Err(TlsError::OutOfMemory)
        } else {
            Ok(())
        }
    }
}
impl<C: TlsCipherSuite, V: TlsVerifier<C>> TlsVerifier<C> for HeapCheckedVerifier<V> {
    fn set_hostname_verification(&mut self, host: &str) -> Result<(), TlsError> {
        self.inner.set_hostname_verification(host)
    }
    fn verify_certificate(
        &mut self,
        transcript: &C::Hash,
        cert: CertificateRef,
    ) -> Result<(), TlsError> {
        self.check()?;
        self.inner.verify_certificate(transcript, cert)
    }
    fn verify_signature(&mut self, signature: CertificateVerifyRef) -> Result<(), TlsError> {
        self.check()?;
        self.inner.verify_signature(signature)
    }
}
