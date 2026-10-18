# Offline ZTM TLS heap fixture

`ztm-chain.bin` contains the public certificate chain returned by
`www.ztm.poznan.pl:443` with SNI `www.ztm.poznan.pl`, captured on 2026-10-18 UTC
using `openssl s_client -showcerts`. It contains no private key or credentials.
The root DER exactly matches `firmware/certs/certum-trusted-root.der`.

The payload uses TLS 1.3 Certificate framing: empty request context, 24-bit
certificate-list length, then leaf/intermediate/root entries, each prefixed by
its 24-bit DER length and followed by an empty two-byte extension list. Tests
parse this framing through the real `embedded-tls` API, including the owned
certificate retained for CertificateVerify. Tests never contact the server.

Certificates:

- `*.ztm.poznan.pl`, valid 2026-06-23 through 2027-01-08.
- Certum OV TLS G2 R39 CA, valid 2024-06-18 through 2039-06-05.
- Certum Trusted Root CA, valid 2018-03-16 through 2043-03-16.

The fixed verification clock is 2026-10-19 UTC, keeping CI independent of real
certificate expiry. Negative cases change the host, clock, certificate
signature and handshake signature. The heap probe uses the same first-fit
allocator crate as `esp-alloc`; host ABI results are not physical C3 peaks.

SHA-256: `309f54b14b12d57b58e5ce8f26cdafb0afbea0648d7d545e96cae95b1c1e9ee0`.
