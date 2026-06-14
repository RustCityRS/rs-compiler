//! Dependency-free heap profiling, compiled only under `--features memprof`.
//!
//! Wraps the System allocator to track live bytes, the global high-water mark,
//! and a running allocation count. Used to attribute the compiler's memory
//! footprint to individual pipeline phases (see `mem_mark` in `lib.rs`). The
//! atomics add a small per-allocation cost, so this is a profiling build only —
//! the default release build is untouched.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

pub struct Counting;

#[inline]
fn bump_peak(cur: usize) {
    let mut peak = PEAK.load(Ordering::Relaxed);
    while cur > peak {
        match PEAK.compare_exchange_weak(peak, cur, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => peak = observed,
        }
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let cur = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            bump_peak(cur);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            if new_size >= layout.size() {
                let cur = CURRENT.fetch_add(new_size - layout.size(), Ordering::Relaxed)
                    + (new_size - layout.size());
                bump_peak(cur);
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        p
    }
}

pub fn current_bytes() -> usize {
    CURRENT.load(Ordering::Relaxed)
}
pub fn peak_bytes() -> usize {
    PEAK.load(Ordering::Relaxed)
}
pub fn alloc_count() -> usize {
    ALLOCS.load(Ordering::Relaxed)
}
