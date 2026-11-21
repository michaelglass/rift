//! The Roc core's mode table against the Rust core's (`rift_core::modes`), exhaustively: every
//! initiative, against every combination of axis states (the pointer idle, pressing or dragging,
//! times two states for each other axis). The verdicts must be identical, refusing axis included.

use rift_core::Pointer;
use rift_core::modes::{
    FocusClaim, INITIATIVES, Menu, MissionControl, Modal, Modes, Topology, UserSpaces, Viewport,
    Wake, WorkspaceSwitch,
};
use rift_core_roc::AllocationCount;

#[test]
fn the_roc_core_decides_every_mode_verdict_as_the_rust_core_does() {
    let before = AllocationCount::now();
    let mut checked = 0;
    let mut refused = 0;
    for pointer in [Pointer::Idle, Pointer::Pressing(17), Pointer::Dragging(23)] {
        for bits in 0u32..512 {
            let on = |i: u32| bits & (1 << i) != 0;
            let modes = Modes {
                pointer,
                menu: if on(0) { Menu::Open } else { Menu::Closed },
                mission_control: if on(1) {
                    MissionControl::Active
                } else {
                    MissionControl::Inactive
                },
                modal: if on(2) {
                    Modal::Frontmost
                } else {
                    Modal::Clear
                },
                viewport: if on(3) {
                    Viewport::Gesturing
                } else {
                    Viewport::Free
                },
                focus_claim: if on(4) {
                    FocusClaim::PausesHover
                } else {
                    FocusClaim::Free
                },
                user_spaces: if on(5) {
                    UserSpaces::None
                } else {
                    UserSpaces::Shown
                },
                wake: if on(6) {
                    Wake::AwaitingInput
                } else {
                    Wake::UserPresent
                },
                workspace_switch: if on(7) {
                    WorkspaceSwitch::Switching
                } else {
                    WorkspaceSwitch::Idle
                },
                topology: if on(8) {
                    Topology::Unsettled
                } else {
                    Topology::Settled
                },
            };
            for initiative in INITIATIVES {
                let rust = modes.verdict(initiative);
                assert_eq!(
                    rift_core_roc::modes::verdict(modes, initiative),
                    rust,
                    "{initiative:?} with {modes:?}"
                );
                checked += 1;
                refused += usize::from(rust != rift_core::modes::Verdict::Admitted);
            }
        }
    }
    assert_eq!(checked, 3 * 512 * 9);
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    eprintln!("modes: all {checked} verdicts identical ({refused} refused)");
}
