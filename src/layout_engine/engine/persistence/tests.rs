use objc2_core_foundation::{CGPoint, CGRect, CGSize};

use super::matcher::MatchScope;
use super::*;
use crate::actor::app::WindowInfo;
use crate::common::config::LayoutMode;
use crate::layout_engine::systems::WindowQuery;
use crate::layout_engine::{LayoutEvent, LayoutSystemKind, StartingLayout};
use crate::model::VirtualWorkspace;
use crate::model::virtual_workspace::WorkspaceTransferReason;
use crate::model::window_store::OwnedWindows;
use crate::sys::window_server::WindowServerId;

type Inventory<'a> = &'a [(&'a str, &'a [SpaceId])];

const LEGACY_V2: &str = r#"{"workspace_layouts":(map:{((901),(idx:3,version:1)):(configurations:{(width:1200,height:800):(idx:1,version:1)},active_size:(width:1200,height:800),last_saved:None),((901),(idx:1,version:1)):(configurations:{(width:1600,height:1000):(idx:2,version:1),(width:1200,height:800):(idx:1,version:1)},active_size:(width:1200,height:800),last_saved:Some((idx:1,version:1))),((901),(idx:2,version:1)):(configurations:{(width:1200,height:800):(idx:1,version:1)},active_size:(width:1200,height:800),last_saved:None),((901),(idx:4,version:1)):(configurations:{(width:1200,height:800):(idx:1,version:1)},active_size:(width:1200,height:800),last_saved:None)}),"schema_version":2,"floating":(floating_windows:[(pid:9,idx:3)],last_floating_focus:None),"floating_positions":(positions:{((901),(idx:1,version:1),(pid:9,idx:3)):(origin:(x:12.0,y:34.0),size:(width:400.0,height:300.0))}),"virtual_workspace_manager":(workspaces:[(value:None,version:0),(value:Some((name:"Main",space:(901),last_focused:Some((pid:9,idx:2)),layout_system:stack((inner:(tree:(map:(map:[(value:None,version:0),(value:Some((parent:None,prev_sibling:None,next_sibling:None,first_child:Some((idx:2,version:3)),last_child:Some((idx:3,version:1)))),version:1),(value:Some((parent:Some((idx:1,version:1)),prev_sibling:None,next_sibling:Some((idx:3,version:1)),first_child:None,last_child:None)),version:3),(value:Some((parent:Some((idx:1,version:1)),prev_sibling:Some((idx:2,version:3)),next_sibling:None,first_child:None,last_child:None)),version:1),(value:Some((parent:None,prev_sibling:None,next_sibling:None,first_child:Some((idx:6,version:3)),last_child:Some((idx:5,version:3)))),version:1),(value:Some((parent:Some((idx:4,version:1)),prev_sibling:Some((idx:6,version:3)),next_sibling:None,first_child:None,last_child:None)),version:3),(value:Some((parent:Some((idx:4,version:1)),prev_sibling:None,next_sibling:Some((idx:5,version:3)),first_child:None,last_child:None)),version:3)]),data:(selection:(nodes:[(value:None,version:0),(value:Some((selected_child:(idx:3,version:1),stop_here:false)),version:1),(value:None,version:0),(value:None,version:0),(value:Some((selected_child:(idx:6,version:3),stop_here:false)),version:1)]),layout:(info:[(value:None,version:0),(value:Some((size:0.0,total:1.0,kind:vertical_stack,last_ungrouped_kind:horizontal,is_fullscreen:false,is_fullscreen_within_gaps:false)),version:1),(value:Some((size:0.5,total:0.0,kind:horizontal,last_ungrouped_kind:horizontal,is_fullscreen:false,is_fullscreen_within_gaps:false)),version:3),(value:Some((size:0.5,total:0.0,kind:horizontal,last_ungrouped_kind:horizontal,is_fullscreen:false,is_fullscreen_within_gaps:false)),version:1),(value:Some((size:0.0,total:2.0,kind:vertical_stack,last_ungrouped_kind:horizontal,is_fullscreen:false,is_fullscreen_within_gaps:false)),version:1),(value:Some((size:1.0,total:0.0,kind:horizontal,last_ungrouped_kind:horizontal,is_fullscreen:false,is_fullscreen_within_gaps:false)),version:3),(value:Some((size:1.0,total:0.0,kind:horizontal,last_ungrouped_kind:horizontal,is_fullscreen:false,is_fullscreen_within_gaps:false)),version:3)]),window:(windows:[(value:None,version:0),(value:None,version:0),(value:Some((pid:9,idx:1)),version:3),(value:Some((pid:9,idx:2)),version:1),(value:None,version:0),(value:Some((pid:9,idx:2)),version:3),(value:Some((pid:9,idx:1)),version:3)],window_nodes:{(pid:9,idx:1):([(layout:(idx:1,version:1),node:(idx:2,version:3)),(layout:(idx:2,version:1),node:(idx:6,version:3))]),(pid:9,idx:2):([(layout:(idx:1,version:1),node:(idx:3,version:1)),(layout:(idx:2,version:1),node:(idx:5,version:3))])}))),layout_roots:[(value:None,version:0),(value:Some((Some((idx:1,version:1)),"layout_root")),version:1),(value:Some((Some((idx:4,version:1)),"layout_root")),version:1)]),default_orientation:perpendicular)),layout_mode:stack)),version:1),(value:Some((name:"Development",space:(901),last_focused:None,layout_system:stack((inner:(tree:(map:(map:[(value:None,version:0),(value:Some((parent:None,prev_sibling:None,next_sibling:None,first_child:None,last_child:None)),version:1)]),data:(selection:(nodes:[(value:None,version:0)]),layout:(info:[(value:None,version:0),(value:Some((size:0.0,total:0.0,kind:vertical_stack,last_ungrouped_kind:horizontal,is_fullscreen:false,is_fullscreen_within_gaps:false)),version:1)]),window:(windows:[(value:None,version:0)],window_nodes:{}))),layout_roots:[(value:None,version:0),(value:Some((Some((idx:1,version:1)),"layout_root")),version:1)]),default_orientation:perpendicular)),layout_mode:stack)),version:1),(value:Some((name:"Communication",space:(901),last_focused:None,layout_system:stack((inner:(tree:(map:(map:[(value:None,version:0),(value:Some((parent:None,prev_sibling:None,next_sibling:None,first_child:None,last_child:None)),version:1)]),data:(selection:(nodes:[(value:None,version:0)]),layout:(info:[(value:None,version:0),(value:Some((size:0.0,total:0.0,kind:vertical_stack,last_ungrouped_kind:horizontal,is_fullscreen:false,is_fullscreen_within_gaps:false)),version:1)]),window:(windows:[(value:None,version:0)],window_nodes:{}))),layout_roots:[(value:None,version:0),(value:Some((Some((idx:1,version:1)),"layout_root")),version:1)]),default_orientation:perpendicular)),layout_mode:stack)),version:1),(value:Some((name:"Utilities",space:(901),last_focused:None,layout_system:stack((inner:(tree:(map:(map:[(value:None,version:0),(value:Some((parent:None,prev_sibling:None,next_sibling:None,first_child:None,last_child:None)),version:1)]),data:(selection:(nodes:[(value:None,version:0)]),layout:(info:[(value:None,version:0),(value:Some((size:0.0,total:0.0,kind:vertical_stack,last_ungrouped_kind:horizontal,is_fullscreen:false,is_fullscreen_within_gaps:false)),version:1)]),window:(windows:[(value:None,version:0)],window_nodes:{}))),layout_roots:[(value:None,version:0),(value:Some((Some((idx:1,version:1)),"layout_root")),version:1)]),default_orientation:perpendicular)),layout_mode:stack)),version:1)],workspaces_by_space:{(901):[(idx:1,version:1),(idx:2,version:1),(idx:3,version:1),(idx:4,version:1)]},active_workspace_per_space:{(901):(None,(idx:1,version:1))},workspace_counter:1),"space_display_map":{},"display_last_space":{},"persisted_windows":{(pid:9,idx:2):(window_server_id:Some(2),title:Some("WindowId { pid: 9, idx: 2 }"),width:400.0,height:300.0,app_id:Some("test.app")),(pid:9,idx:1):(window_server_id:Some(1),title:Some("WindowId { pid: 9, idx: 1 }"),width:400.0,height:300.0,app_id:Some("test.app")),(pid:9,idx:3):(window_server_id:Some(3),title:Some("WindowId { pid: 9, idx: 3 }"),width:400.0,height:300.0,app_id:Some("test.app"))},"saved_active_space":None}"#;

fn win(pid: pid_t, idx: u32) -> WindowId { WindowId::new(pid, idx) }

fn ws(engine: &LayoutEngine, space: SpaceId, index: usize) -> VirtualWorkspaceId {
    engine.workspaces.workspace_ids(space)[index]
}

fn evidence(window: WindowId) -> WindowFingerprint {
    WindowFingerprint {
        window_server_id: Some(10_000 + window.idx.get()),
        title: Some(format!("window {}", window.idx.get())),
        width: 800.0,
        height: 600.0,
        app_id: Some(format!("com.example.app{}", window.pid)),
    }
}

fn live_state(window: WindowId) -> WindowState {
    let saved = evidence(window);
    let frame = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(saved.width, saved.height));
    WindowState::from(WindowInfo {
        has_native_tabs: false,
        is_standard: true,
        is_root: true,
        is_minimized: false,
        is_resizable: true,
        min_size: None,
        max_size: None,
        title: saved.title.unwrap(),
        frame,
        sys_id: saved.window_server_id.map(WindowServerId::new),
        bundle_id: saved.app_id,
        path: None,
        ax_role: None,
        ax_subrole: None,
    })
}

fn fresh(
    settings: &VirtualWorkspaceSettings,
    spaces: &[(SpaceId, &str)],
) -> (LayoutEngine, OwnedWindows) {
    let mut engine = LayoutEngine::new(settings, &LayoutSettings::default());
    let mut store = OwnedWindows::default();
    let all: Vec<_> = spaces.iter().map(|(space, _)| *space).collect();
    engine
        .workspaces
        .ensure_visible_spaces(&mut store.view_mut(), &all, all.len(), |_| None);
    for (space, display) in spaces {
        let size = CGSize::new(1200.0, 800.0);
        let _ = engine.handle_event(&mut store.view_mut(), LayoutEvent::SpaceExposed(*space, size));
        engine.update_space_display(*space, Some(*display));
    }
    (engine, store)
}

fn open(
    engine: &mut LayoutEngine,
    store: &mut WindowsMut<'_>,
    id: VirtualWorkspaceId,
    window: WindowId,
) {
    let space = engine.workspaces[id].space;
    store.insert_manageable_window(window, live_state(window));
    assert!(engine.workspaces.assign_window_to_workspace(store, space, window, id));
    engine.add_window_to_layout(store.read(), space, window);
}

/// A file saved by a session with `windows` tiled on each Space's first workspace.
fn session(settings: &VirtualWorkspaceSettings, spaces: &[(SpaceId, &str, &[WindowId])]) -> String {
    let displays: Vec<_> = spaces.iter().map(|(space, display, _)| (*space, *display)).collect();
    let (mut engine, mut store) = fresh(settings, &displays);
    for (space, _, windows) in spaces {
        let id = ws(&engine, *space, 0);
        for window in *windows {
            open(&mut engine, &mut store.view_mut(), id, *window);
        }
    }
    engine.serialize_to_string(store.view())
}

/// `saved` loaded, and the core seeded from its topology.
fn load(saved: &str) -> (LayoutEngine, OwnedWindows) {
    let StartingLayout { engine, topology } = LayoutEngine::deserialize_from_str(saved).unwrap();
    (engine, OwnedWindows::joining(&topology))
}

fn boot(settings: &VirtualWorkspaceSettings, saved: &str) -> (LayoutEngine, OwnedWindows) {
    let (mut engine, mut store) = load(saved);
    engine.await_topology();
    engine
        .finish_loading(&mut store.view_mut(), settings, &LayoutSettings::default())
        .unwrap();
    (engine, store)
}

fn resolve(
    engine: &mut LayoutEngine,
    store: &mut WindowsMut<'_>,
    current: &[(SpaceId, &str)],
    inventory: Inventory<'_>,
) {
    let size = CGSize::new(1200.0, 800.0);
    let current: Vec<_> = current.iter().map(|(s, d)| (*s, (*d).to_owned(), size)).collect();
    let inventory = inventory.iter().map(|(d, s)| ((*d).to_owned(), s.to_vec())).collect();
    engine.resolve_saved_spaces(store, &current, current.len(), &inventory);
}

fn observe(
    engine: &mut LayoutEngine,
    store: &mut WindowsMut<'_>,
    space: SpaceId,
    window: WindowId,
    live: &WindowFingerprint,
) -> bool {
    engine.is_restoring()
        && engine.reconcile_saved_window(store, window, live, MatchScope::Space(space))
}

fn restore(
    engine: &mut LayoutEngine,
    store: &mut WindowsMut<'_>,
    file: &str,
    scope: RestoreScope,
    active_space: SpaceId,
    source: RestoreSource,
) -> anyhow::Result<RestoreReport> {
    restore_on(engine, store, file, scope, active_space, source, &[active_space])
}

fn restore_on(
    engine: &mut LayoutEngine,
    store: &mut WindowsMut<'_>,
    file: &str,
    scope: RestoreScope,
    active_space: SpaceId,
    source: RestoreSource,
    visible: &[SpaceId],
) -> anyhow::Result<RestoreReport> {
    let path = std::env::temp_dir().join(format!(
        "rift-persistence-test-{}-{}.ron",
        std::process::id(),
        SAVE_TEMP_COUNTER.fetch_add(1, AtomicOrdering::Relaxed),
    ));
    fs::write(&path, file).unwrap();
    let request = RestoreRequest { scope, active_space, source };
    let result =
        engine.restore_layout(path.clone(), request, visible, store, &LayoutSettings::default());
    let _ = fs::remove_file(path);
    result
}

fn tree(engine: &LayoutEngine, id: VirtualWorkspaceId) -> Vec<WindowId> {
    let entry = &engine.workspaces[id];
    (entry.layout_state.all_layouts())
        .flat_map(|layout| entry.layout_system.windows(layout, WindowQuery::All))
        .collect()
}

fn with_active_space(file: String, space: SpaceId) -> String {
    file.replace(
        r#""saved_active_space":None"#,
        &format!(r#""saved_active_space":Some({})"#, space.get()),
    )
}

/// A file carrying raw floating state, as older versions wrote it.
fn with_floating(
    file: String,
    marked: &[WindowId],
    frames: &[(VirtualWorkspaceId, WindowId, CGRect)],
) -> String {
    let mut floating = FloatingManager::new();
    marked.iter().for_each(|window| floating.add_floating(*window));
    let mut positions = FloatingPositionStore::default();
    frames
        .iter()
        .for_each(|(id, window, frame)| positions.store(*id, *window, *frame));
    file.replacen(
        r#""floating":(floating_windows:[],last_floating_focus:None),"floating_positions":(positions:{})"#,
        &format!(
            r#""floating":{},"floating_positions":{}"#,
            ron::ser::to_string(&floating).unwrap(),
            ron::ser::to_string(&positions).unwrap(),
        ),
        1,
    )
}

fn assert_rejected(file: &str, message: &str) {
    let error = LayoutEngine::deserialize_from_str(file).err().expect("file must be rejected");
    assert!(error.to_string().contains(message), "{error}");
}

#[test]
fn restores_legacy_slots_and_atomically_installs_workspace_contents() {
    let per_display = VirtualWorkspaceSettings::default();
    let shared = VirtualWorkspaceSettings {
        mode: WorkspaceMode::Global,
        default_workspace_count: 2,
        workspace_names: vec!["One".into(), "Two".into()],
        ..Default::default()
    };
    let [a, b, c] = [900, 901, 902].map(SpaceId::new);
    let (w1, w2, w3) = (win(7, 1), win(7, 2), win(7, 3));
    let other = win(8, 4);
    let settings = LayoutSettings::default();

    // Format: every layout system keeps its exact structure, in files of every age.
    for (mode, tag) in [
        (LayoutMode::Traditional, "traditional"),
        (LayoutMode::Bsp, "bsp"),
        (LayoutMode::Stack, "stack"),
        (LayoutMode::MasterStack, "master_stack"),
        (LayoutMode::Scrolling, "scrolling"),
    ] {
        let (mut engine, mut store) = fresh(&per_display, &[(a, "x")]);
        let id = ws(&engine, a, 0);
        let _ = engine.switch_workspace_layout_mode(store.view(), a, id, mode);
        [w1, w2, w3]
            .iter()
            .for_each(|w| open(&mut engine, &mut store.view_mut(), id, *w));
        let saved = engine.serialize_to_string(store.view());
        let (loaded, _) = load(&saved);
        let ron_of = |engine: &LayoutEngine| ron::ser::to_string(&engine.workspaces[id]).unwrap();
        assert_eq!(ron_of(&loaded), ron_of(&engine));

        let system = VirtualWorkspace::create_layout_system(mode, &settings);
        let current = ron::ser::to_string(&system).unwrap();
        let prefix = format!("{tag}((");
        let legacy = format!("(kind:\"{tag}\",{})", &current[prefix.len()..current.len() - 2]);
        let migrated = super::snapshot::migrate_legacy_layout_system_tags(&legacy).unwrap();
        let _: LayoutSystemKind = ron::from_str(&migrated).unwrap();
    }
    let empty = session(&per_display, &[(a, "x", &[])]);
    LayoutEngine::deserialize_from_str(&empty.replacen("\"schema_version\":5,", "", 1)).unwrap();
    assert_rejected(
        &empty.replacen("\"schema_version\":5", "\"schema_version\":6", 1),
        "newer than supported",
    );
    for version in [None, Some(1), Some(2)] {
        let header = version.map(|v| format!("\"schema_version\":{v},")).unwrap_or_default();
        let old = LEGACY_V2.replacen("\"schema_version\":2,", &header, 1);
        let (mut loaded, joined) = load(&old);
        let id = loaded.workspaces.active_workspace(SpaceId::new(901)).unwrap();
        assert_eq!(loaded.workspaces[id].layout_mode, LayoutMode::Stack);
        assert_eq!(loaded.workspaces[id].layout_state.all_layouts().count(), 2);
        assert_eq!(loaded.workspaces[id].last_focused(), Some(win(9, 2)));
        loaded.workspaces[id].ensure_layout_for_size(CGSize::new(1600.0, 1000.0));
        assert_eq!(loaded.workspaces[id].layout_state.all_layouts().count(), 1);
        let canonical = loaded.serialize_to_string(joined.view());
        assert!(!canonical.contains("\"workspace_layouts\":"));
        let (reloaded, _) = load(&canonical);
        assert_eq!(reloaded.persistence.pending[&win(9, 3)].workspace, id);
        assert_eq!(
            reloaded.persistence.pending[&win(9, 3)].frame,
            Some(CGRect::new(CGPoint::new(12.0, 34.0), CGSize::new(400.0, 300.0))),
        );
    }
    let (mut engine, mut store) = fresh(&shared, &[(a, "x"), (b, "y")]);
    let moved = engine.workspaces.active_workspace(a).unwrap();
    engine.workspaces.transfer_workspace(
        &mut store.view_mut(),
        moved,
        b,
        None,
        WorkspaceTransferReason::DisplayDeparture,
    );
    let replacement = engine.workspaces.active_workspace(a);
    let saved = engine.serialize_to_string(store.view());
    let current = &saved[activations_at(&saved)];
    let old = ron::ser::to_string(&HashMap::from_iter([
        (a, (None::<VirtualWorkspaceId>, moved)),
        (b, (None, engine.workspaces.active_workspace(b).unwrap())),
    ]))
    .unwrap();
    let file = saved.replacen(current, &old, 1);
    let (loaded, _) = load(&file);
    assert_eq!(loaded.workspaces.active_workspace(a), replacement);
    assert_eq!(
        loaded.workspaces.active_workspace_per_space()[&a].remembered(),
        Some(moved)
    );

    // Validation: a bad file fails whole and leaves the live engine exactly as it was.
    let good = session(&per_display, &[(a, "x", &[w1])]);
    let id = load(&good).0.workspaces.workspace_ids(a)[0];
    let negative = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(-1.0, 1.0));
    assert_rejected(
        &good.replacen("width:800.0", "width:-1.0", 1),
        "invalid persisted size",
    );
    assert_rejected(
        &with_floating(good.clone(), &[], &[(id, w1, negative)]),
        "invalid floating frame",
    );
    assert_rejected(
        &LEGACY_V2.replacen("((901),", "((902),", 1),
        "invalid workspace layouts",
    );
    assert_rejected(
        &LEGACY_V2.replacen("(idx:3,version:1)", "(idx:999999,version:1)", 1),
        "invalid workspace layouts",
    );
    assert_rejected(
        &LEGACY_V2.replacen(
            "selected_child:(idx:3,version:1)",
            "selected_child:(idx:999999,version:1)",
            1,
        ),
        "layout selection references an invalid child",
    );
    let layout = good.find("layout:Some((idx:").unwrap();
    let version = layout + good[layout..].find("version:1").unwrap();
    let mut dangling = good.clone();
    dangling.replace_range(version..version + 9, "version:9999");
    assert_rejected(&dangling, "references missing layout");
    let (mut engine, mut store) = boot(&per_display, &good);
    resolve(&mut engine, &mut store.view_mut(), &[(a, "x")], &[("x", &[a])]);
    let before = engine.serialize_to_string(store.view());
    for bad in [
        good[..good.len() - 20].to_owned(),
        good.replacen("\"schema_version\":5", "\"schema_version\":99", 1),
        good.replacen("width:800.0", "width:-1.0", 1),
    ] {
        for scope in [RestoreScope::Space, RestoreScope::Workspace] {
            let source = RestoreSource::CurrentSpace;
            assert!(restore(&mut engine, &mut store.view_mut(), &bad, scope, a, source).is_err());
            assert_eq!(engine.serialize_to_string(store.view()), before);
        }
    }

    // Restore: matched windows take saved slots, current windows stay, the rest leave no trace.
    let portable = with_active_space(
        session(&per_display, &[(a, "x", &[w1, w2]), (b, "y", &[other])]),
        a,
    );
    let (mut engine, mut store) = fresh(&per_display, &[(c, "z")]);
    let target = ws(&engine, c, 0);
    let name = engine.workspaces[target].name.clone();
    let absent = win(9, 1);
    let outsider = win(7, 2);
    open(&mut engine, &mut store.view_mut(), target, absent);
    open(&mut engine, &mut store.view_mut(), target, w1);
    let sibling = ws(&engine, c, 1);
    open(&mut engine, &mut store.view_mut(), sibling, outsider);
    let report = restore(
        &mut engine,
        &mut store.view_mut(),
        &portable,
        RestoreScope::Workspace,
        c,
        RestoreSource::SavedActiveSpace,
    )
    .unwrap();
    assert_eq!(
        (report.matched, report.unmatched, report.workspaces_replaced),
        (1, 1, 1)
    );
    assert_eq!(tree(&engine, target), [w1, absent]);
    assert_eq!(
        tree(&engine, sibling),
        [outsider],
        "a sibling's live window is not consumed"
    );
    assert_eq!(
        engine.workspaces[target].name, name,
        "contents replace, identity stays"
    );
    assert_eq!(engine.workspaces[target].space, c);
    assert!(engine.persistence.pending.is_empty());
}

#[test]
fn single_display_restore_keeps_the_surviving_desktops_catalog() {
    let settings = VirtualWorkspaceSettings {
        mode: WorkspaceMode::Global,
        default_workspace_count: 2,
        workspace_names: vec!["1".into(), "2".into()],
        ..Default::default()
    };
    let (old, current) = (SpaceId::new(801), SpaceId::new(802));
    for exposed in [true, false] {
        let mut engine = LayoutEngine::new(&settings, &LayoutSettings::default());
        let mut store = OwnedWindows::default();
        engine
            .workspaces
            .ensure_visible_spaces(&mut store.view_mut(), &[current], 1, |_| None);
        engine.workspaces.ensure_space_initialized(&mut store.view_mut(), old);
        if exposed {
            engine.update_space_display(old, Some("display"));
        }
        engine.update_space_display(current, Some("display"));
        let expected = engine.workspaces.workspace_ids(current);
        let saved = engine.serialize_to_string(store.view());
        let (mut engine, mut store) = boot(&settings, &saved);
        resolve(&mut engine, &mut store.view_mut(), &[(current, "display")], &[(
            "display",
            &[current],
        )]);
        assert_eq!(engine.workspaces.workspace_ids(current), expected);
    }
}

#[test]
fn global_single_display_restore_stays_in_the_windows_native_space() {
    let settings = VirtualWorkspaceSettings {
        mode: WorkspaceMode::Global,
        ..Default::default()
    };
    let (a, b) = (SpaceId::new(801), SpaceId::new(802));
    let window = win(80, 1);
    let file = session(&settings, &[(a, "display", &[window])]);
    for current in [a, b] {
        let (mut engine, mut store) = boot(&settings, &file);
        resolve(&mut engine, &mut store.view_mut(), &[(current, "display")], &[(
            "display",
            &[a, b],
        )]);
        engine
            .workspaces
            .ensure_visible_spaces(&mut store.view_mut(), &[current], 1, |_| None);
        let target = ws(&engine, current, 0);
        open(&mut engine, &mut store.view_mut(), target, window);
        assert_eq!(
            observe(
                &mut engine,
                &mut store.view_mut(),
                current,
                window,
                &evidence(window)
            ),
            current == a
        );
        assert_eq!(store.view().workspace_for_window(window), Some(target));
    }
}

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self { Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1) }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize { (self.next() % n as u64) as usize }
}

type CoreActivation = rift_core::workspaces::Activation<VirtualWorkspaceId>;

/// An activation's shape: what it shows, and whether it has history and memory.
fn shape(activation: &CoreActivation) -> &'static str {
    use rift_core::workspaces::Activation::{Showing, Vacant};
    match activation {
        Showing {
            previous: None,
            remembered: None,
            ..
        } => "showing",
        Showing {
            previous: Some(_),
            remembered: None,
            ..
        } => "showing, previous",
        Showing {
            previous: None,
            remembered: Some(_),
            ..
        } => "showing, remembered",
        Showing {
            previous: Some(_),
            remembered: Some(_),
            ..
        } => "showing, previous, remembered",
        Vacant { remembered: None } => "vacant",
        Vacant { remembered: Some(_) } => "vacant, remembered",
    }
}

/// The core's topology: its catalog in order, and its activations by Space.
fn topology_of(
    core: &crate::roc_twin::TwinnedWorkspaces,
) -> (
    Vec<(VirtualWorkspaceId, SpaceId)>,
    Vec<(SpaceId, CoreActivation)>,
) {
    let mut activations = core.activations().to_vec();
    activations.sort_by_key(|&(space, _)| space);
    (core.catalog().to_vec(), activations)
}

/// A store driven by `seed` through the workspace store's topology writers: workspaces created,
/// shown from another Space, remembered (also on a Space never exposed), moved between Spaces,
/// and Spaces removed.
fn drawn(settings: &VirtualWorkspaceSettings, seed: u64) -> (LayoutEngine, OwnedWindows) {
    let mut rng = Rng::new(seed);
    let all = [
        (SpaceId::new(301), "d1"),
        (SpaceId::new(302), "d2"),
        (SpaceId::new(303), "d3"),
    ];
    let spaces = &all[..1 + rng.below(all.len())];
    let (mut engine, mut store) = fresh(settings, spaces);
    let unexposed = SpaceId::new(304);
    let space = |rng: &mut Rng| spaces[rng.below(spaces.len())].0;
    for _ in 0..rng.below(14) {
        let catalog: Vec<_> = engine.workspaces.catalog().collect();
        let Some(&workspace) = catalog.get(rng.below(catalog.len().max(1))) else {
            break;
        };
        let windows = &mut store.view_mut();
        match rng.below(6) {
            0 => {
                let on = space(&mut rng);
                let _ = engine.workspaces.create_workspace(windows, on, None);
            }
            1 => {
                let from = space(&mut rng);
                engine.workspaces.set_active_workspace_from(windows, from, workspace);
            }
            2 => {
                let on = space(&mut rng);
                engine.workspaces.remember_on(windows, on, workspace);
            }
            3 => engine.workspaces.remember_on(windows, unexposed, workspace),
            4 => {
                let gone = if rng.below(2) == 0 {
                    unexposed
                } else {
                    space(&mut rng)
                };
                engine
                    .workspaces
                    .remove_space(windows, gone, |_| rift_core::workspaces::Successor::Unassign);
            }
            _ => {
                let to = space(&mut rng);
                engine.workspaces.transfer_workspace(
                    windows,
                    workspace,
                    to,
                    None,
                    WorkspaceTransferReason::DisplayDeparture,
                );
            }
        }
    }
    (engine, store)
}

/// The saved form carries the topology whole: over drawn stores in both modes, the core a load
/// seeds holds exactly the saving core's catalog and activations, and the draws reach every
/// activation shape in each mode.
#[test]
fn a_saved_topology_seeds_the_core_it_was_saved_from() {
    let per_display = VirtualWorkspaceSettings::default();
    let shared = VirtualWorkspaceSettings {
        mode: WorkspaceMode::Global,
        default_workspace_count: 2,
        ..Default::default()
    };
    for (mode, settings) in [("per-display", &per_display), ("shared", &shared)] {
        let mut shapes = std::collections::BTreeSet::new();
        for seed in 0..300 {
            let (engine, store) = drawn(settings, seed);
            let saving = topology_of(&store.core);
            shapes.extend(saving.1.iter().map(|(_, activation)| shape(activation)));
            let saved = engine.serialize_to_string(store.view());
            let (_, joined) = load(&saved);
            assert_eq!(topology_of(&joined.core), saving, "{mode}, seed {seed}");
        }
        assert_eq!(shapes.len(), 6, "{mode}: the draws reach only {shapes:?}");
    }
}

/// The fixture states: one drawn store in each mode (four activations, four shapes), with a
/// window tiled on the first workspace.
fn fixture_state(mode: &str) -> (LayoutEngine, OwnedWindows) {
    let (settings, seed) = match mode {
        "per-display" => (VirtualWorkspaceSettings::default(), 14),
        _ => (
            VirtualWorkspaceSettings {
                mode: WorkspaceMode::Global,
                default_workspace_count: 2,
                ..Default::default()
            },
            28,
        ),
    };
    let (mut engine, mut store) = drawn(&settings, seed);
    let first = engine.workspaces.catalog().next().unwrap();
    open(&mut engine, &mut store.view_mut(), first, win(7, 1));
    (engine, store)
}

/// Where `file` holds its saved activations (the map, braces and all).
fn activations_at(file: &str) -> std::ops::Range<usize> {
    let key = "active_workspace_per_space:";
    let start = file.find(key).expect("a saved store has its activations") + key.len();
    let mut depth = 0;
    let end = start
        + file[start..]
            .char_indices()
            .find_map(|(at, c)| {
                depth += match c {
                    '{' | '(' | '[' => 1,
                    '}' | ')' | ']' => -1,
                    _ => 0,
                };
                (depth == 0).then_some(at + 1)
            })
            .unwrap();
    start..end
}

/// `file` with its saved activations in Space order: written from a hash map, their order is the
/// map's, so two writers agree on the activations as a map, not on their order.
fn activations_in_order(file: &str) -> String {
    let at = activations_at(file);
    let activations: BTreeMap<SpaceId, ron::Value> = ron::from_str(&file[at.clone()]).unwrap();
    format!(
        "{}{}{}",
        &file[..at.start],
        ron::ser::to_string(&activations).unwrap(),
        &file[at.end..]
    )
}

/// Saved files stay compatible both ways, through fixtures the serializer wrote before the core
/// took the saved topology at the load boundary: each loads into the core it was saved from, and
/// the same state is saved as the same file (its activations as a map), so a build from before
/// reads what this one writes.
#[test]
fn files_saved_before_the_topology_boundary_load_and_are_written_alike() {
    for (mode, file) in [
        (
            "per-display",
            include_str!("fixtures/saved-before-2a-per-display.ron"),
        ),
        ("shared", include_str!("fixtures/saved-before-2a-shared.ron")),
    ] {
        let (engine, store) = fixture_state(mode);
        let (_, joined) = load(file);
        assert_eq!(topology_of(&joined.core), topology_of(&store.core), "{mode}");
        let saved = engine.serialize_to_string(store.view());
        assert_eq!(
            activations_in_order(&saved),
            activations_in_order(file),
            "{mode}"
        );
    }
}

/// A saved topology that names what it can't hold is refused whole at the load boundary.
#[test]
fn a_saved_topology_naming_what_it_cannot_hold_is_refused() {
    let file = include_str!("fixtures/saved-before-2a-per-display.ron");
    let showing = "(301):(current:Some((idx:1,version:1)),previous:None,remembered:None)";
    let remembering = "(304):(current:None,previous:None,remembered:Some((idx:6,version:1)))";
    assert!(file.contains(showing) && file.contains(remembering));
    for (broken, refusal) in [
        (
            file.replacen(showing, &showing.replace("idx:1,", "idx:5,"), 1),
            "native space 301 has an invalid active workspace",
        ),
        (
            file.replacen(
                showing,
                &showing.replace("previous:None", "previous:Some((idx:60,version:1))"),
                1,
            ),
            "native space 301 has an invalid previous workspace",
        ),
        (
            file.replacen(remembering, &remembering.replace("idx:6,", "idx:60,"), 1),
            "native space 304 remembers a missing workspace",
        ),
        (
            file.replacen(&format!("{showing},"), "", 1)
                .replacen(&format!(",{showing}"), "", 1),
            "workspace VirtualWorkspaceId(1v1) for native space 301 is not indexed",
        ),
    ] {
        assert_rejected(&broken, &format!("invalid workspace topology: {refusal}"));
    }
}
