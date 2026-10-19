#[path = "../src/tls_cipher.rs"]
mod tls_cipher;
use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use embedded_tls::TlsCipherSuite;

#[test]
fn standard_tls_chacha_suite_matches_rfc8439_and_rejects_tampering() {
    // RFC8439§2.8.2: https://www.rfc-editor.org/rfc/rfc8439#section-2.8.2
    type Cipher = <tls_cipher::CipherSuite as TlsCipherSuite>::Cipher;
    assert_eq!(tls_cipher::CipherSuite::CODE_POINT, 0x1303);
    let key: [u8; 32] = core::array::from_fn(|i| 0x80 + i as u8);
    let nonce = [7, 0, 0, 0, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47].into();
    let aad = [
        0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
    ];
    let plain = *b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let cipher = Cipher::new(&key.into());
    let mut encrypted = plain;
    let tag = cipher
        .encrypt_in_place_detached(&nonce, &aad, &mut encrypted)
        .unwrap();
    assert_eq!(
        &tag[..],
        &[
            0x1a, 0xe1, 0x0b, 0x59, 0x4f, 0x09, 0xe2, 0x6a, 0x7e, 0x90, 0x2e, 0xcb, 0xd0, 0x60,
            0x06, 0x91
        ]
    );
    for kind in 0..4 {
        let mut data = encrypted;
        let mut changed_aad = aad;
        let mut changed_tag = tag;
        let mut changed_nonce = nonce;
        match kind {
            0 => data[0] ^= 1,
            1 => changed_aad[0] ^= 1,
            2 => changed_tag[0] ^= 1,
            _ => changed_nonce[0] ^= 1,
        }
        assert!(
            cipher
                .decrypt_in_place_detached(&changed_nonce, &changed_aad, &mut data, &changed_tag)
                .is_err()
        );
    }
    cipher
        .decrypt_in_place_detached(&nonce, &aad, &mut encrypted, &tag)
        .unwrap();
    assert_eq!(encrypted, plain);
}
