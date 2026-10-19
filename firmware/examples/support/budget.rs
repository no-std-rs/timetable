//! Host diagnostic only. Hard 64 KiB first-fit heap; no System fallback while
//! tracking. Host OS/socket buffers and native code are outside this arena.
use linked_list_allocator::Heap;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::UnsafeCell,
    ptr::NonNull,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
#[repr(align(16))]
struct Arena([u8; 65536]);
struct Storage(UnsafeCell<Arena>);
unsafe impl Sync for Storage {}
static ARENA: Storage = Storage(UnsafeCell::new(Arena([0; 65536])));
fn base() -> *mut u8 {
    unsafe { core::ptr::addr_of_mut!((*ARENA.0.get()).0).cast() }
}
static TRACK: AtomicBool = AtomicBool::new(false);
struct Meter {
    heap: Heap,
    peak: usize,
}
static METER: Mutex<Meter> = Mutex::new(Meter {
    heap: Heap::empty(),
    peak: 0,
});
pub struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            let mut m = METER.lock().unwrap();
            let p = m
                .heap
                .allocate_first_fit(layout)
                .map_or(core::ptr::null_mut(), |p| p.as_ptr());
            m.peak = m.peak.max(m.heap.used());
            p
        } else {
            unsafe { System.alloc(layout) }
        }
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        if (base() as usize..base() as usize + 65536).contains(&(p as usize)) {
            unsafe {
                METER
                    .lock()
                    .unwrap()
                    .heap
                    .deallocate(NonNull::new(p).unwrap(), layout)
            };
        } else {
            unsafe { System.dealloc(p, layout) };
        }
    }
}
pub fn available() -> usize {
    METER.lock().unwrap().heap.free()
}
pub fn start(reserve: usize) -> NonNull<u8> {
    assert!(reserve < 65536);
    {
        let mut m = METER.lock().unwrap();
        unsafe { m.heap.init(base(), 65536) };
    }
    TRACK.store(true, Ordering::Relaxed);
    NonNull::new(unsafe { Allocator.alloc(Layout::from_size_align(reserve, 4).unwrap()) }).unwrap()
}
pub fn finish(reserve: usize, pointer: NonNull<u8>) -> (usize, usize) {
    TRACK.store(false, Ordering::Relaxed);
    unsafe {
        Allocator.dealloc(
            pointer.as_ptr(),
            Layout::from_size_align(reserve, 4).unwrap(),
        )
    };
    let m = METER.lock().unwrap();
    (m.peak, m.heap.used())
}
