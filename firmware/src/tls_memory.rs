//! Admission check around the unchanged TLS certificate verifier.
use embedded_tls::{
    CertificateEntryRef, CertificateRef, CertificateVerifyRef, TlsCipherSuite, TlsError,
    TlsVerifier,
};

// The pinned verifier's actual ZTM chain fits in a 24 KiB first-fit arena in
// the offline regression. This is admission headroom, not a radio peak bound:
// other radio tasks can still allocate, so their queues are bounded separately.
pub const VERIFY_HEAP_BYTES: usize = 24 * 1024;

pub struct HeapCheckedVerifier<V> {
    inner: V,
    available: fn() -> usize,
    redundant_root: Option<&'static [u8]>,
}
impl<V> HeapCheckedVerifier<V> {
    pub fn new(inner: V, available: fn() -> usize) -> Self {
        Self {
            inner,
            available,
            redundant_root: None,
        }
    }
    /// The server may append a cross-signed version of our pinned trust anchor.
    /// The linear embedded-tls verifier otherwise tries to verify that extra
    /// certificate with the same anchor's key. Omit only an exact known DER
    /// match; intermediates and leaf still verify against the pinned anchor.
    pub fn with_redundant_root(mut self, root: Option<&'static [u8]>) -> Self {
        self.redundant_root = root;
        self
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
        mut cert: CertificateRef,
    ) -> Result<(), TlsError> {
        self.check()?;
        if cert.entries.len() > 1
            && let Some(CertificateEntryRef::X509(last)) = cert.entries.last()
            && self.redundant_root == Some(*last)
        {
            cert.entries.pop();
        }
        self.inner.verify_certificate(transcript, cert)
    }
    fn verify_signature(&mut self, signature: CertificateVerifyRef) -> Result<(), TlsError> {
        self.check()?;
        self.inner.verify_signature(signature)
    }
}
