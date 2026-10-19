//! QEMU virt RV32 instruction-cost probe, not ESP32 cycle/power emulation.
//! UART/exit addresses and entry stack belong only to the QEMU virt machine.
#![no_std]
#![no_main]
use aes_gcm::aead::{AeadInPlace, KeyInit};
use core::{
    arch::{asm, global_asm},
    fmt::{self, Write},
    hint::black_box,
};
global_asm!(
    ".section .text.boot",
    ".global _start",
    "_start:",
    "li sp, 0x81000000",
    "call main",
    "1: j 1b"
);
struct Uart;
impl Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            unsafe {
                while (0x10000005 as *const u8).read_volatile() & 32 == 0 {}
                (0x10000000 as *mut u8).write_volatile(b);
            }
        }
        Ok(())
    }
}
fn instret() -> u64 {
    unsafe {
        let (hi, lo, again): (u32, u32, u32);
        asm!("1: rdinstreth {hi}","rdinstret {lo}","rdinstreth {again}","bne {hi}, {again}, 1b",hi=out(reg)hi,lo=out(reg)lo,again=out(reg)again,options(nostack));
        let _ = again;
        ((hi as u64) << 32) | lo as u64
    }
}
#[inline(never)]
fn bench<C: AeadInPlace<NonceSize = aes_gcm::aead::consts::U12> + KeyInit>(name: &str, key: &[u8]) {
    let mut data = [0u8; 16384];
    let aad = [0x17, 3, 3, 0x40, 0x10];
    for i in 0..4 {
        data.fill(0x5a);
        let nonce_bytes = [i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let nonce: aes_gcm::aead::Nonce<C> = nonce_bytes.into();
        let start = instret();
        let cipher = C::new_from_slice(black_box(key)).unwrap();
        let tag = cipher
            .encrypt_in_place_detached(&nonce, &aad, black_box(&mut data))
            .unwrap();
        let enc = instret() - start;
        let start = instret();
        let cipher = C::new_from_slice(black_box(key)).unwrap();
        cipher
            .decrypt_in_place_detached(&nonce, &aad, black_box(&mut data), &tag)
            .unwrap();
        let dec = instret() - start;
        assert!(data.iter().all(|b| *b == 0x5a));
        writeln!(
            Uart,
            "{name} record{i} encrypt_inst={enc} decrypt_inst={dec}"
        )
        .unwrap();
    }
}
#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    writeln!(
        Uart,
        "RV32IMC AEAD 16384B,5B AAD; opt=s; instructions not ESP32 timing"
    )
    .unwrap();
    bench::<aes_gcm::Aes128Gcm>("AES128GCM", &[7; 16]);
    bench::<chacha20poly1305::ChaCha20Poly1305>("CHACHA20POLY1305", &[7; 32]);
    bench_csv();
    bench_zip();
    unsafe { (0x100000 as *mut u32).write_volatile(0x5555) }
    loop {
        unsafe { asm!("wfi") }
    }
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let _ = writeln!(Uart, "PANIC {info}");
    unsafe { (0x100000 as *mut u32).write_volatile(0x3333) }
    loop {}
}

struct Bytes(&'static [u8]);
impl embedded_io_async::ErrorType for Bytes {
    type Error = core::convert::Infallible;
}
impl embedded_io_async::Read for Bytes {
    async fn read(&mut self, o: &mut [u8]) -> Result<usize, Self::Error> {
        let n = o.len().min(self.0.len());
        o[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        Ok(n)
    }
}
fn ready<F: core::future::Future>(f: F) -> F::Output {
    let mut f = core::pin::pin!(f);
    match f.as_mut().poll(&mut core::task::Context::from_waker(
        core::task::Waker::noop(),
    )) {
        core::task::Poll::Ready(v) => v,
        _ => panic!("pending"),
    }
}
macro_rules! csvbench {
    ($krate:ident,$name:literal) => {{
        let start = instret();
        let (rows, sum) = ready(async {
            let mut reader = $krate::csv::Reader::<_>::new(
                Bytes(include_bytes!("../stop_times.csv")),
                $krate::Table::StopTimes,
            );
            let mut count = 0;
            let mut sum = 0;
            while let Some(row) = reader.next().await.unwrap() {
                sum += black_box(row.required($krate::csv::Column::TripId).unwrap()).len();
                count += 1;
            }
            (count, sum)
        });
        let used = instret() - start;
        writeln!(
            Uart,
            "CSV {} rows={rows} sum={sum} retired_inst={used}",
            $name
        )
        .unwrap();
    }};
}
fn bench_csv() {
    for _ in 0..3 {
        csvbench!(gtfs, "current");
    }
}

#[allow(dead_code)]
#[path = "../../../firmware/src/cooperative.rs"]
mod cooperative;

fn scheduled<F: core::future::Future>(f: F) -> (F::Output, u64, u64, u32) {
    let mut f = core::pin::pin!(f);
    let mut total = 0;
    let mut longest = 0;
    let mut polls = 0;
    loop {
        let start = instret();
        let result = f.as_mut().poll(&mut core::task::Context::from_waker(
            core::task::Waker::noop(),
        ));
        let used = instret() - start;
        total += used;
        longest = longest.max(used);
        polls += 1;
        if let core::task::Poll::Ready(value) = result {
            return (value, total, longest, polls);
        }
    }
}
async fn parse_zip<R: embedded_io_async::Read>(source: R) -> (usize, usize) {
    let mut rows = gtfs::csv::Reader::<_>::new(source, gtfs::Table::StopTimes);
    let mut count = 0;
    let mut sum = 0;
    while let Some(row) = rows.next().await.unwrap() {
        sum += black_box(row.required(gtfs::csv::Column::TripId).unwrap()).len();
        count += 1;
    }
    (count, sum)
}
fn bench_zip() {
    for (fixture, bytes, expected_rows, expected_sum) in [
        (
            "normal",
            include_bytes!("../stop_times.zip").as_slice(),
            512,
            6130,
        ),
        (
            "repeated",
            include_bytes!("../stop_times-repeated.zip").as_slice(),
            8192,
            98080,
        ),
    ] {
        for decoded in [false, true] {
            let mut workspace = gtfs::zip::Workspace::new();
            let ((rows, sum), total, longest, polls) = scheduled(async {
                let compressed = cooperative::Cooperative::new(Bytes(bytes));
                let mut zip = gtfs::zip::Reader::new(compressed, &mut workspace);
                assert!(zip.next_entry().await.unwrap().is_some());
                if decoded {
                    parse_zip(cooperative::Cooperative::new(zip)).await
                } else {
                    parse_zip(zip).await
                }
            });
            assert_eq!((rows, sum), (expected_rows, expected_sum));
            writeln!(Uart,"ZIP fixture={fixture} decoded={decoded} rows={rows} sum={sum} total_inst={total} max_poll_inst={longest} polls={polls}").unwrap();
        }
    }
}
