//! Exercise the production verifier in a bounded first-fit arena. On an arena
//! miss only this host probe falls back to System so it can report the failure
//! instead of aborting the whole test process. Production has no such fallback.
use embedded_tls::{
    Aes128GcmSha256, Certificate, CertificateRef, CertificateVerifyRef, Sha256, SignatureScheme,
    TlsClock, TlsError, TlsVerifier, pki::CertVerifier,
};
use linked_list_allocator::Heap;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::{Cell, UnsafeCell},
    ptr::NonNull,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
#[path = "../src/tls_memory.rs"]
mod tls_memory;
use tls_memory::{HeapCheckedVerifier, VERIFY_HEAP_BYTES};

thread_local! { static TRACK: Cell<bool> = const { Cell::new(false) }; }
#[repr(align(16))]
struct Arena([u8; 65536]);
struct Storage(UnsafeCell<Arena>);
// The arena is only initialized/allocated/freed under METER's lock; all previous
// allocations must be freed before starting another measurement.
unsafe impl Sync for Storage {}
static ARENA: Storage = Storage(UnsafeCell::new(Arena([0; 65536])));
fn arena_ptr() -> *mut u8 {
    // Compute the field address without creating a reference to mutable storage.
    unsafe { std::ptr::addr_of_mut!((*ARENA.0.get()).0).cast() }
}
struct Meter {
    heap: Heap,
    peak: usize,
    misses: usize,
}
static METER: Mutex<Meter> = Mutex::new(Meter {
    heap: Heap::empty(),
    peak: 0,
    misses: 0,
});
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            let mut meter = METER.lock().unwrap();
            if let Ok(ptr) = meter.heap.allocate_first_fit(layout) {
                meter.peak = meter.peak.max(meter.heap.used());
                return ptr.as_ptr();
            }
            meter.misses += 1;
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let base = arena_ptr() as usize;
        if (base..base + 65536).contains(&(ptr as usize)) {
            unsafe {
                METER
                    .lock()
                    .unwrap()
                    .heap
                    .deallocate(NonNull::new(ptr).unwrap(), layout)
            };
        } else {
            unsafe { System.dealloc(ptr, layout) };
        }
    }
}
#[global_allocator]
static ALLOC: Allocator = Allocator;
static TIME: AtomicU64 = AtomicU64::new(1792368000);
struct Clock;
impl TlsClock for Clock {
    fn now() -> Option<u64> {
        Some(TIME.load(Ordering::Relaxed))
    }
}
const CHAIN: &[u8] = include_bytes!("fixtures/tls/ztm-chain.bin");
const ROOT: &[u8] = include_bytes!("../certs/certum-trusted-root.der");
fn chain(bytes: &[u8]) -> CertificateRef<'_> {
    CertificateRef::parse(&mut bytes.into()).unwrap()
}
fn available() -> usize {
    METER.lock().unwrap().heap.free()
}
fn measure<T>(bytes: usize, operation: impl FnOnce() -> T) -> (T, usize, usize) {
    {
        let mut meter = METER.lock().unwrap();
        assert_eq!(meter.heap.used(), 0);
        // init may only run once per Heap, even after all allocations are freed.
        meter.heap = Heap::empty();
        unsafe { meter.heap.init(arena_ptr(), bytes) };
        meter.peak = 0;
        meter.misses = 0;
    }
    TRACK.with(|on| on.set(true));
    let result = operation();
    TRACK.with(|on| on.set(false));
    let meter = METER.lock().unwrap();
    assert_eq!(meter.heap.used(), 0, "verifier leaked arena allocations");
    (result, meter.peak, meter.misses)
}
fn verifier() -> CertVerifier<'static, Aes128GcmSha256, Clock, 8192> {
    let mut verifier = CertVerifier::new(Certificate::X509(ROOT));
    verifier
        .set_hostname_verification("www.ztm.poznan.pl")
        .unwrap();
    verifier
}
#[test]
fn real_ztm_rsa_chain_respects_heap_admission_and_rejects_invalid_certificates() {
    let hash = Sha256::default();
    let observed_free = 65536 - 46376;
    let (result, _, misses) = measure(observed_free, || {
        verifier().verify_certificate(&hash, chain(CHAIN))
    });
    assert!(result.is_ok()); // System fallback lets the probe finish and report arena misses.
    assert!(
        misses > 0,
        "the old physical radio baseline must reproduce exhaustion"
    );
    let (result, peak, misses) = measure(observed_free, || {
        HeapCheckedVerifier::new(verifier(), available).verify_certificate(&hash, chain(CHAIN))
    });
    assert!(matches!(result, Err(TlsError::OutOfMemory)));
    assert_eq!(
        (peak, misses),
        (0, 0),
        "admission must reject before RSA allocations"
    );
    for _ in 0..3 {
        let (result, peak, misses) = measure(VERIFY_HEAP_BYTES, || {
            HeapCheckedVerifier::new(verifier(), available).verify_certificate(&hash, chain(CHAIN))
        });
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            misses, 0,
            "real verifier exceeded its bounded first-fit arena"
        );
        println!(
            "ZTM RSA chain: {peak} / {VERIFY_HEAP_BYTES} B first-fit heap, no retained allocations"
        );
    }
    let mut checked = HeapCheckedVerifier::new(verifier(), available);
    assert!(
        measure(VERIFY_HEAP_BYTES, || checked
            .verify_certificate(&hash, chain(CHAIN)))
        .0
        .is_ok()
    );
    let signature = || CertificateVerifyRef {
        signature_scheme: SignatureScheme::RsaPssRsaeSha256,
        signature: &[0; 256],
    };
    let (result, peak, misses) = measure(observed_free, || checked.verify_signature(signature()));
    assert!(matches!(result, Err(TlsError::OutOfMemory)));
    assert_eq!((peak, misses), (0, 0));
    let (result, _, misses) = measure(VERIFY_HEAP_BYTES, || checked.verify_signature(signature()));
    assert!(matches!(result, Err(TlsError::InvalidSignature)));
    assert_eq!(misses, 0);
    let (result, _, misses) = measure(VERIFY_HEAP_BYTES, || {
        let mut v = HeapCheckedVerifier::new(verifier(), available);
        v.set_hostname_verification("another.example").unwrap();
        v.verify_certificate(&hash, chain(CHAIN))
    });
    assert!(result.is_err());
    assert_eq!(misses, 0);
    TIME.store(1700000000, Ordering::Relaxed);
    let (result, _, misses) = measure(VERIFY_HEAP_BYTES, || {
        HeapCheckedVerifier::new(verifier(), available).verify_certificate(&hash, chain(CHAIN))
    });
    assert!(result.is_err());
    assert_eq!(misses, 0);
    TIME.store(1792368000, Ordering::Relaxed);
    let mut corrupt = CHAIN.to_vec();
    let signature_byte = corrupt.len() - 3;
    corrupt[signature_byte] ^= 1;
    let (result, _, misses) = measure(VERIFY_HEAP_BYTES, || {
        HeapCheckedVerifier::new(verifier(), available).verify_certificate(&hash, chain(&corrupt))
    });
    assert!(result.is_err());
    assert_eq!(misses, 0);
    mirror_chains(&hash);
}

fn mirror_chains(hash: &Sha256) {
    for (host, root, cross, bytes) in [
        (
            "mobilitydatabase.org",
            &include_bytes!("../certs/gts-root-r4.der")[..],
            &include_bytes!("../certs/mobility-cross.der")[..],
            &include_bytes!("fixtures/tls/mobility-chain.bin")[..],
        ),
        (
            "files.mobilitydatabase.org",
            &include_bytes!("../certs/gts-root-r1.der")[..],
            &include_bytes!("../certs/mobility-files-cross.der")[..],
            &include_bytes!("fixtures/tls/mobility-files-chain.bin")[..],
        ),
    ] {
        let make = || {
            let mut verifier =
                CertVerifier::<Aes128GcmSha256, Clock, 8192>::new(Certificate::X509(root));
            verifier.set_hostname_verification(host).unwrap();
            HeapCheckedVerifier::new(verifier, available).with_redundant_root(Some(cross))
        };
        let (result, peak, misses) = measure(VERIFY_HEAP_BYTES, || {
            make().verify_certificate(hash, chain(bytes))
        });
        assert!(result.is_ok(), "{host}: {result:?}");
        assert_eq!(misses, 0);
        println!(
            "{host}: {peak}/{VERIFY_HEAP_BYTES}B first-fit; modern anchor and exact cross-cert normalization"
        );
        let (result, _, misses) = measure(VERIFY_HEAP_BYTES, || {
            make()
                .with_redundant_root(None)
                .verify_certificate(hash, chain(bytes))
        });
        assert!(
            result.is_err(),
            "fixture must reproduce linear-chain incompatibility"
        );
        assert_eq!(misses, 0);
        for wrong_host in [false, true] {
            let (result, _, misses) = measure(VERIFY_HEAP_BYTES, || {
                let mut v = make();
                if wrong_host {
                    v.set_hostname_verification("wrong.example").unwrap();
                } else {
                    TIME.store(1700000000, Ordering::Relaxed);
                }
                v.verify_certificate(hash, chain(bytes))
            });
            TIME.store(1792368000, Ordering::Relaxed);
            assert!(result.is_err());
            assert_eq!(misses, 0);
        }
        // Tamper separately with the leaf, intermediate and redundant cross cert.
        let mut offset = 4;
        for _ in 0..3 {
            let size = ((bytes[offset] as usize) << 16)
                | ((bytes[offset + 1] as usize) << 8)
                | bytes[offset + 2] as usize;
            let mut altered = bytes.to_vec();
            altered[offset + 3 + size - 1] ^= 1;
            let (result, _, misses) = measure(VERIFY_HEAP_BYTES, || {
                make().verify_certificate(hash, chain(&altered))
            });
            assert!(
                result.is_err(),
                "{host}: certificate signature corruption accepted"
            );
            assert_eq!(misses, 0);
            offset += 3 + size + 2;
        }
    }
}
