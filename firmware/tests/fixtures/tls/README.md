# Offline TLS heap fixtures

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

## MobilityDatabase mirror

`mobility-chain.bin` and `mobility-files-chain.bin` contain the public chains
from `mobilitydatabase.org:443` and `files.mobilitydatabase.org:443`, respectively,
with matching SNI, captured using `openssl s_client -showcerts` on 2026-10-19 UTC.
They use the same TLS 1.3 framing and fixed verification date as the ZTM fixture.
No private key or credentials are included.

The website uses GTS Root R4; the file host uses GTS Root R1. The self-signed
pinned roots in `firmware/certs/` come from Google's official
[root bundle](https://pki.goog/roots.pem), retrieved on the same date.
Both servers append a cross-signed version of their GTS root. `embedded-tls`
0.19 verifies a linear chain: the exact bundled redundant last certificate is
omitted from the parsed verification view so the remaining chain terminates at
our modern pinned self-signed root. The raw TLS transcript remains unchanged.
No legacy GlobalSign trust anchor is added. An altered cross-certificate is
never omitted; the normal verifier rejects it. Hostname, time, certificate and
handshake signature checks remain enabled.

Tests use the real parser/verifier in the same 24 KiB first-fit arena and check
wrong hostname/time and corrupted leaf, intermediate and cross-certificate.
The original unnormalized chain fails against the modern root, reproducing the
linear-chain compatibility issue. The file host's RSA chain peaks at 22,328 B
on the host, with no retained allocations. Live probes separately complete
verified handshakes through this adapter; these results do not measure C3 RAM.

| Public file | SHA-256 |
| --- | --- |
| `mobility-chain.bin` | `52189f1ad9d49e5764fdd7c02aff163199270ee668bea9de6ac240168d2a81b4` |
| `mobility-files-chain.bin` | `fc23c688ab74e055c58a7a260eb780848ea5b482eecc6f284dfe5446310fa3ad` |
| `gts-root-r1.der` | `d947432abde7b7fa90fc2e6b59101b1280e0e1c7e4e40fa3c6887fff57a7f4cf` |
| `gts-root-r4.der` | `349dfa4058c5e263123b398ae795573c4e1313c83fe68f93556cd5e8031b3c7d` |
| `mobility-cross.der` | `76b27b80a58027dc3cf1da68dac17010ed93997d0b603e2fadbe85012493b5a7` |
| `mobility-files-cross.der` | `3ee0278df71fa3c125c4cd487f01d774694e6fc57e0cd94c24efd769133918e5` |
