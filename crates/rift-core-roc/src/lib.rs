//! Rift's core written in Roc, stepped like `rift_core::RustCore`.
//!
//! The Roc app (`roc/core`) is linked in as a static archive built by `build.rs`. This crate is
//! its host: it supplies Roc's allocator and crash hooks, and [`RocCore`] steps it. Events go in
//! and effects come out as words (`rift_core::plain::wire`), so the glue
//! (`roc_platform_abi.rs`, regenerated with `roc/glue.sh` on every nightly bump) is three
//! functions on word lists whatever the core grows. The effect words are copied out of Roc
//! memory and released at once, so the platform never holds a Roc allocation.
//!
//! Empty without the `roc` feature, so rift's workspace builds without the Roc nightly.
#![cfg(feature = "roc")]

#[allow(clippy::all, non_camel_case_types, unused, unsafe_op_in_unsafe_fn)]
#[path = "roc_platform_abi.rs"]
mod abi;

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

use rift_core::plain::{Plain, wire};
use rift_core::{Effect, Event};

// ---------------------------------------------------------------------------------------------
// Roc's runtime hooks. Roc runs on one thread (the caller's), so the counters are thread-local.

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static DEALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

/// Allocations Roc made and freed on this thread so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocationCount {
    pub allocated: u64,
    pub freed: u64,
}

impl AllocationCount {
    pub fn now() -> Self {
        AllocationCount {
            allocated: ALLOCATIONS.with(Cell::get),
            freed: DEALLOCATIONS.with(Cell::get),
        }
    }

    pub fn live(self) -> i64 { self.allocated as i64 - self.freed as i64 }
}

fn out_of_memory() -> ! {
    eprintln!("roc: out of memory");
    std::process::abort()
}

// macOS malloc aligns to 16 bytes, Roc's largest alignment on arm64.
#[unsafe(no_mangle)]
pub extern "C" fn roc_alloc(length: usize, alignment: usize) -> NonNull<c_void> {
    debug_assert!(alignment <= 16);
    ALLOCATIONS.with(|count| count.set(count.get() + 1));
    NonNull::new(unsafe { malloc(length) }).unwrap_or_else(|| out_of_memory())
}

#[unsafe(no_mangle)]
pub extern "C" fn roc_realloc(
    ptr: *mut c_void,
    length: usize,
    alignment: usize,
) -> NonNull<c_void> {
    debug_assert!(alignment <= 16);
    NonNull::new(unsafe { realloc(ptr, length) }).unwrap_or_else(|| out_of_memory())
}

#[unsafe(no_mangle)]
pub extern "C" fn roc_dealloc(ptr: *mut c_void, _alignment: usize) {
    DEALLOCATIONS.with(|count| count.set(count.get() + 1));
    unsafe { free(ptr) }
}

fn message(bytes: *const u8, len: usize) -> String {
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(bytes, len) }).into_owned()
}

#[unsafe(no_mangle)]
pub extern "C" fn roc_dbg(bytes: *const u8, len: usize) {
    eprintln!("[roc dbg] {}", message(bytes, len));
}

#[unsafe(no_mangle)]
pub extern "C" fn roc_expect_failed(bytes: *const u8, len: usize) {
    eprintln!("[roc expect failed] {}", message(bytes, len));
}

/// A core crash is fatal: Roc cannot unwind, and the platform restores window state on restart.
#[unsafe(no_mangle)]
pub extern "C" fn roc_crashed(bytes: *const u8, len: usize) {
    eprintln!("[roc crashed] {}", message(bytes, len));
    std::process::abort()
}

extern "C" fn host_alloc(_: *mut abi::RocHost, length: usize, alignment: usize) -> NonNull<c_void> {
    roc_alloc(length, alignment)
}

extern "C" fn host_dealloc(_: *mut abi::RocHost, ptr: *mut c_void, alignment: usize) {
    roc_dealloc(ptr, alignment)
}

extern "C" fn host_realloc(
    _: *mut abi::RocHost,
    ptr: *mut c_void,
    length: usize,
    alignment: usize,
) -> NonNull<c_void> {
    roc_realloc(ptr, length, alignment)
}

extern "C" fn host_dbg(_: *mut abi::RocHost, bytes: *const u8, len: usize) { roc_dbg(bytes, len) }

extern "C" fn host_expect_failed(_: *mut abi::RocHost, bytes: *const u8, len: usize) {
    roc_expect_failed(bytes, len)
}

extern "C" fn host_crashed(_: *mut abi::RocHost, bytes: *const u8, len: usize) {
    roc_crashed(bytes, len)
}

/// The hooks the glue's helpers (list building and releasing) use: the same allocator.
fn host() -> abi::RocHost {
    abi::RocHost {
        env: std::ptr::null_mut(),
        roc_alloc: host_alloc,
        roc_dealloc: host_dealloc,
        roc_realloc: host_realloc,
        roc_dbg: host_dbg,
        roc_expect_failed: host_expect_failed,
        roc_crashed: host_crashed,
    }
}

// ---------------------------------------------------------------------------------------------
// The core.

/// The Roc core. Holds the model's box, which only Roc can read or free; it is moved through
/// every step and never shared, so Roc can update it in place.
pub struct RocCore {
    model: abi::RocBox,
    host: abi::RocHost,
}

// SAFETY: the model's box is owned by this `RocCore` alone: Roc keeps no reference to it
// between steps and nothing else holds one, so moving the core to another thread (rift builds
// its reactor on one thread and runs it on another) moves the only owner. Roc's refcounts are
// not atomic, so the core is not `Sync`: one thread steps it at a time, which `&mut self` on
// `step` already requires.
unsafe impl Send for RocCore {}

impl Default for RocCore {
    fn default() -> Self {
        RocCore {
            model: unsafe { abi::roc_init() },
            host: host(),
        }
    }
}

impl RocCore {
    /// One step on words: an event's words in, the effect list's words out.
    pub fn step_words(&mut self, event: &[u64]) -> Vec<u64> {
        // Roc takes ownership of the event list and releases it.
        let event = unsafe { abi::RocListWith::from_slice(event, &self.host) };
        let stepped = unsafe { abi::roc_step(self.model, event) };
        self.model = stepped.model;
        let effects = stepped.effects.as_slice().to_vec();
        unsafe { stepped.effects.decref(&self.host) };
        effects
    }

    /// One step, as `RustCore::step`. The Roc app only ever writes effects the wire can read:
    /// a word list it cannot is a bug in one of the two codecs, so it panics.
    pub fn step(&mut self, event: Event<Plain>) -> Vec<Effect<Plain>> {
        let words = self.step_words(&wire::encode_event(event));
        wire::decode_effects(&words).unwrap_or_else(|error| panic!("roc core effects: {error:?}"))
    }
}

impl Drop for RocCore {
    fn drop(&mut self) { unsafe { abi::roc_release_model(self.model) } }
}
