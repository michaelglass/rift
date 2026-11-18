//! Rift's core written in Roc, behind `rift_core::Core`.
//!
//! The Roc app (`roc/core`) is linked in as a static archive built by `build.rs`. This crate is
//! its host: it supplies Roc's allocator and crash hooks, and [`RocCore`] adapts the generated
//! glue (`roc_platform_abi.rs`, regenerated with `roc/glue.sh` on every nightly bump) to the
//! core's public types. The glue ABI never leaves this crate: events go in as `rift_core` plain
//! data and effects come out as `rift_core` plain data, copied out of Roc memory and released
//! at once, so the platform never holds a Roc allocation.

#[allow(clippy::all, non_camel_case_types, unused, unsafe_op_in_unsafe_fn)]
#[path = "roc_platform_abi.rs"]
mod abi;

use std::cell::Cell;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::ptr::NonNull;

use rift_core::plain::{Nanos, Plain, Rect, WindowId};
use rift_core::{
    Answer, Core, Effect, Event, Initiative, InputClass, Note, Observation, Pointer, Want, Write,
};

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
// Plain data to the glue's types and back.

fn window_in(window: WindowId) -> abi::EventWantWindow {
    abi::EventWantWindow {
        idx: window.idx,
        pid: window.pid,
    }
}

fn window_out(window: abi::EffectWriteWindow) -> WindowId {
    WindowId {
        pid: window.pid,
        idx: window.idx,
    }
}

fn rect_in(rect: Rect) -> abi::EventWantDesired {
    abi::EventWantDesired {
        h: rect.h,
        w: rect.w,
        x: rect.x,
        y: rect.y,
    }
}

fn rect_out(rect: abi::EffectWriteFrame) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
    }
}

/// The Roc core's platform predates the mode table's `Arranging` and `Hover` input classes and
/// the `FollowActivation` initiative (the Roc port is paused). On the pointer's row each has
/// exactly `UserInput(Steering)`'s rule, so it is asked as that; `step` puts the original
/// initiative back on the answer.
fn initiative_in(initiative: Initiative) -> abi::EventGate {
    use abi::ArrangeOrRetireWindowsOrSwitchWorkspaceFocusOrUserInputOrWarpFocusTag as Tag;
    let mut gate: abi::EventGate = unsafe { std::mem::zeroed() };
    gate.tag = match initiative {
        Initiative::UserInput(class) => {
            gate.payload.user_input = ManuallyDrop::new(match class {
                InputClass::Steering | InputClass::Arranging | InputClass::Hover => {
                    abi::InertOrSteering::Steering
                }
                InputClass::Inert => abi::InertOrSteering::Inert,
            });
            Tag::UserInput
        }
        Initiative::FollowActivation => {
            gate.payload.user_input = ManuallyDrop::new(abi::InertOrSteering::Steering);
            Tag::UserInput
        }
        Initiative::Arrange => Tag::Arrange,
        Initiative::RetireWindows => Tag::RetireWindows,
        Initiative::SwitchWorkspaceFocus => Tag::SwitchWorkspaceFocus,
        Initiative::WarpFocus => Tag::WarpFocus,
    };
    gate
}

fn initiative_out(gate: &abi::EventGate) -> Initiative {
    use abi::ArrangeOrRetireWindowsOrSwitchWorkspaceFocusOrUserInputOrWarpFocusTag as Tag;
    match gate.tag {
        Tag::UserInput => {
            Initiative::UserInput(match *unsafe { &gate.payload.user_input }.clone() {
                abi::InertOrSteering::Steering => InputClass::Steering,
                abi::InertOrSteering::Inert => InputClass::Inert,
            })
        }
        Tag::Arrange => Initiative::Arrange,
        Tag::RetireWindows => Initiative::RetireWindows,
        Tag::SwitchWorkspaceFocus => Initiative::SwitchWorkspaceFocus,
        Tag::WarpFocus => Initiative::WarpFocus,
    }
}

/// The event in the glue's layout. A `Tick`'s list is allocated with Roc's allocator; Roc owns
/// it once the event is passed in.
fn event_in(event: Event<Plain>, host: &abi::RocHost) -> abi::Event {
    use abi::EventTag as Tag;
    let mut out: abi::Event = unsafe { std::mem::zeroed() };
    out.tag = match event {
        Event::Pointer(pointer) => {
            use abi::DraggingOrIdleOrPressingTag as P;
            let mut slot: abi::EventPointer = unsafe { std::mem::zeroed() };
            slot.tag = match pointer {
                Pointer::Idle => P::Idle,
                Pointer::Pressing(window) => {
                    slot.payload.pressing = ManuallyDrop::new(window_in(window));
                    P::Pressing
                }
                Pointer::Dragging(window) => {
                    slot.payload.dragging = ManuallyDrop::new(window_in(window));
                    P::Dragging
                }
            };
            out.payload.pointer = ManuallyDrop::new(slot);
            Tag::Pointer
        }
        Event::Want {
            now,
            window,
            desired,
            known,
            want,
        } => {
            out.payload.want = ManuallyDrop::new(abi::EventWant {
                desired: rect_in(desired),
                known: rect_in(known),
                now: now.0,
                window: window_in(window),
                want: match want {
                    Want::Arrange => abi::ArrangeOrArrangePositionsOrPlace::Arrange,
                    Want::ArrangePositions => {
                        abi::ArrangeOrArrangePositionsOrPlace::ArrangePositions
                    }
                    Want::Place => abi::ArrangeOrArrangePositionsOrPlace::Place,
                },
            });
            Tag::Want
        }
        Event::Follow { window, frame } => {
            out.payload.follow = ManuallyDrop::new(abi::EventFollow {
                frame: rect_in(frame),
                window: window_in(window),
            });
            Tag::Follow
        }
        Event::Answer { now, window, frame } => {
            out.payload.answer = ManuallyDrop::new(abi::EventAnswer {
                frame: rect_in(frame),
                now: now.0,
                window: window_in(window),
            });
            Tag::Answer
        }
        Event::Report { window, frame, known } => {
            out.payload.report = ManuallyDrop::new(abi::EventReport {
                frame: rect_in(frame),
                known: rect_in(known),
                window: window_in(window),
            });
            Tag::Report
        }
        Event::Tick { now, known } => {
            let known: Vec<abi::EventTickKnown> = known
                .into_iter()
                .map(|(window, frame)| abi::EventTickKnown {
                    frame: rect_in(frame),
                    window: window_in(window),
                })
                .collect();
            let known = unsafe { abi::RocListWith::<_, false>::from_slice(&known, host) };
            out.payload.tick = ManuallyDrop::new(abi::EventTick { now: now.0, known });
            Tag::Tick
        }
        Event::Forget { window } => {
            out.payload.forget = ManuallyDrop::new(window_in(window));
            Tag::Forget
        }
        Event::Gate { initiative } => {
            out.payload.gate = ManuallyDrop::new(initiative_in(initiative));
            Tag::Gate
        }
    };
    out
}

fn effect_out(effect: &abi::Effect) -> Effect<Plain> {
    use abi::EffectTag as Tag;
    // SAFETY (each arm): the tag says which payload is initialised.
    unsafe {
        match effect.tag {
            Tag::Write => {
                let write = &*effect.payload.write;
                Effect::Write(Write::from_foreign_core(
                    window_out(write.window),
                    rect_out(write.frame),
                ))
            }
            Tag::Queued => Effect::Queued {
                window: window_out(*effect.payload.queued),
            },
            Tag::Answered => {
                let answered = &*effect.payload.answered;
                Effect::Answered {
                    window: window_out(answered.window),
                    answer: match answered.answer {
                        abi::AckedOrRefusedOrUnasked::Acked => Answer::Acked,
                        abi::AckedOrRefusedOrUnasked::Refused => Answer::Refused,
                        abi::AckedOrRefusedOrUnasked::Unasked => Answer::Unasked,
                    },
                }
            }
            Tag::Late => Effect::Late {
                window: window_out(*effect.payload.late),
            },
            Tag::Observed => {
                let observed = &*effect.payload.observed;
                Effect::Observed {
                    window: window_out(observed.window),
                    observation: match observed.observation {
                        abi::AckOrForeignOrInTransit::Ack => Observation::Ack,
                        abi::AckOrForeignOrInTransit::InTransit => Observation::InTransit,
                        abi::AckOrForeignOrInTransit::Foreign => Observation::Foreign,
                    },
                }
            }
            Tag::Note => {
                let note = &*effect.payload.note;
                Effect::Note {
                    window: window_out(note.window),
                    note: match note.note {
                        abi::DiscardedOrForgottenOrTimedOut::Discarded => Note::Discarded,
                        abi::DiscardedOrForgottenOrTimedOut::TimedOut => Note::TimedOut,
                        abi::DiscardedOrForgottenOrTimedOut::Forgotten => Note::Forgotten,
                    },
                }
            }
            Tag::Gate => {
                let gate = &*effect.payload.gate;
                Effect::Gate {
                    initiative: initiative_out(&gate.initiative),
                    allowed: gate.allowed,
                }
            }
            Tag::Timer => {
                let timer = &*effect.payload.timer;
                Effect::Timer(match timer.tag {
                    abi::ArmOrDisarmTag::Arm => Some(Nanos(*timer.payload.arm)),
                    abi::ArmOrDisarmTag::Disarm => None,
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------

/// The Roc core. Holds the model's box, which only Roc can read or free; it is moved through
/// every step and never shared, so Roc can update it in place.
pub struct RocCore {
    model: abi::RocBox,
    host: abi::RocHost,
}

impl Default for RocCore {
    fn default() -> Self {
        RocCore {
            model: unsafe { abi::roc_init() },
            host: host(),
        }
    }
}

impl Core<Plain> for RocCore {
    fn step(&mut self, event: Event<Plain>) -> Vec<Effect<Plain>> {
        let asked = match &event {
            Event::Gate { initiative } => Some(*initiative),
            _ => None,
        };
        let event = event_in(event, &self.host);
        let stepped = unsafe { abi::roc_step(self.model, event) };
        self.model = stepped.model;
        let effects = stepped
            .effects
            .as_slice()
            .iter()
            .map(effect_out)
            .map(|effect| match (effect, asked) {
                (Effect::Gate { allowed, .. }, Some(initiative)) => {
                    Effect::Gate { initiative, allowed }
                }
                (effect, _) => effect,
            })
            .collect();
        // The effects hold nothing refcounted: releasing the list frees its spine.
        unsafe { stepped.effects.decref(&self.host) };
        effects
    }
}

impl Drop for RocCore {
    fn drop(&mut self) { unsafe { abi::roc_release_model(self.model) } }
}
