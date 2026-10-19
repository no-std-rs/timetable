//! TLS 1.3 ChaCha20-Poly1305/SHA-256 (RFC 8446, 0x1303).
//! RustCrypto AEAD, no allocation. Avoid software AES/GHASH on the C3's RV32 CPU.
use chacha20poly1305::{
    ChaCha20Poly1305,
    aead::consts::{U12, U32},
};
use embedded_tls::{Aes128GcmSha256, Sha256, TlsCipherSuite};

pub struct CipherSuite;
impl TlsCipherSuite for CipherSuite {
    const CODE_POINT: u16 = 0x1303;
    type Cipher = ChaCha20Poly1305;
    type KeyLen = U32;
    type IvLen = U12;
    type Hash = Sha256;
    type LabelBufferSize = <Aes128GcmSha256 as TlsCipherSuite>::LabelBufferSize;
}
