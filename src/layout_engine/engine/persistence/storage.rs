use super::snapshot::PersistedLayout;
use super::*;

/// `at` as a compact UTC timestamp, `YYYYMMDDTHHMMSSZ` (days to a civil date: Howard Hinnant's
/// algorithm; no time crate needed).
fn utc_stamp(at: std::time::SystemTime) -> String {
    let seconds = at.duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// Crash-safe replacement of `path`. Returning success means the complete file reached the
/// filesystem, because SaveAndExit terminates the process immediately afterwards.
fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent)?;
    }
    let (temporary, mut file) = loop {
        let sequence = SAVE_TEMP_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let suffix = format!("{}.{sequence}.tmp", std::process::id());
        let temporary = path.with_extension(match path.extension() {
            Some(extension) => format!("{}.{suffix}", extension.to_string_lossy()),
            None => suffix,
        });
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary) {
            Ok(file) => break (temporary, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        drop(file);
        // The rename is the commit point; syncing the directory makes it durable.
        fs::rename(&temporary, path)?;
        if let Some(parent) = parent {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

impl LayoutEngine {
    /// Check that a layout file can be loaded, without building an engine from it.
    pub fn validate_file(path: PathBuf) -> anyhow::Result<()> {
        PersistedLayout::read(&path).map(|_| ())
    }

    /// Keeps a copy of a restore file that would not load, as `<path>.unreadable-<UTC stamp>`,
    /// before anything can save over it; answers the copy's path. The original is left as it
    /// is (the next save replaces it).
    pub fn set_aside_unreadable(
        path: &Path,
        now: std::time::SystemTime,
    ) -> std::io::Result<PathBuf> {
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".unreadable-{}", utc_stamp(now)));
        let copy = PathBuf::from(name);
        fs::copy(path, &copy)?;
        Ok(copy)
    }

    /// Load the master snapshot used for process startup and report its persisted coverage.
    pub fn load_for_startup_restore(path: PathBuf) -> anyhow::Result<Self> {
        let snapshot = PersistedLayout::read(&path)?;
        let schema_version = snapshot.schema_version;
        let mut engine = snapshot.into_engine();
        engine.await_topology();
        let unavailable_windows = engine.discard_unmatchable_startup_candidates(
            |window, id| {
                crate::sys::window_server::get_window(
                    crate::sys::window_server::WindowServerId::new(id),
                )
                .is_some_and(|info| info.pid == window.pid)
            },
            crate::sys::app::is_bundle_running,
        );
        tracing::info!(
            path = %path.display(),
            schema_version,
            native_spaces = engine.persistence.unresolved_spaces.len(),
            virtual_workspaces = engine.workspaces.workspaces.len(),
            restore_candidates = engine.persistence.pending.len(),
            unavailable_windows_ignored = unavailable_windows,
            "Loaded persisted layout for startup restore"
        );
        Ok(engine)
    }

    /// Hold every saved destination until `resolve_saved_spaces` has proven it.
    pub(super) fn await_topology(&mut self) {
        self.persistence.awaiting_topology = true;
        self.persistence.unresolved_spaces =
            self.workspaces.workspaces.values().map(|workspace| workspace.space).collect();
    }

    pub(crate) fn deserialize_from_str(buf: &str) -> anyhow::Result<Self> {
        Ok(PersistedLayout::parse(buf)?.into_engine())
    }

    /// Capture and atomically save one coherent snapshot, leaving the engine untouched. A file
    /// that would not load is never committed.
    pub fn save_current_layout(
        &self,
        path: PathBuf,
        window_store: &WindowStore,
        active_space: Option<SpaceId>,
    ) -> std::io::Result<()> {
        // Until the topology is known, a live window can sit in a different workspace from its own
        // pending slot, so this state has no one coherent snapshot.
        if self.persistence.awaiting_topology && !self.persistence.pending.is_empty() {
            return Err(std::io::Error::other(
                "saved layout is still waiting for displays",
            ));
        }
        let serialized = self.snapshot_string(window_store, active_space);
        PersistedLayout::parse(&serialized).map_err(|error| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
        })?;
        write_atomically(&path, &serialized)
    }

    pub fn serialize_to_string(&self, window_store: &WindowStore) -> String {
        self.snapshot_string(window_store, None)
    }

    pub fn finish_loading(
        &mut self,
        virtual_workspace_config: &VirtualWorkspaceSettings,
        layout_settings: &LayoutSettings,
        broadcast_tx: Option<BroadcastSender>,
    ) -> Result<(), String> {
        let plan = self.workspaces.plan_settings_names(virtual_workspace_config, true)?;
        self.broadcast_tx = broadcast_tx;
        self.apply_layout_settings(layout_settings);
        self.app_rules = AppRuleEngine::new(&virtual_workspace_config.app_rules);
        self.workspaces.apply_settings(virtual_workspace_config, layout_settings, plan);
        Ok(())
    }
}

impl PersistedLayout {
    pub(super) fn read(path: &Path) -> anyhow::Result<Self> {
        let mut buf = String::new();
        File::open(path)?.read_to_string(&mut buf)?;
        Self::parse(&buf)
    }
}

#[cfg(test)]
mod set_aside_tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    #[test]
    fn a_timestamp_is_utc_to_the_second() {
        assert_eq!(utc_stamp(UNIX_EPOCH), "19700101T000000Z");
        // 2026-11-19 14:15:02 UTC.
        assert_eq!(
            utc_stamp(UNIX_EPOCH + Duration::from_secs(1_795_097_702)),
            "20261119T141502Z"
        );
        // A leap day.
        assert_eq!(
            utc_stamp(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "20000229T000000Z"
        );
    }

    #[test]
    fn an_unreadable_restore_file_is_copied_aside_and_left_until_the_next_save() {
        let dir = std::env::temp_dir().join(format!("rift-unreadable-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("layout.ron");
        fs::write(&path, "not a layout").unwrap();
        assert!(LayoutEngine::load_for_startup_restore(path.clone()).is_err());
        let now = UNIX_EPOCH + Duration::from_secs(1_795_097_702);
        let copy = LayoutEngine::set_aside_unreadable(&path, now).unwrap();
        assert_eq!(copy, dir.join("layout.ron.unreadable-20261119T141502Z"));
        assert_eq!(fs::read_to_string(&copy).unwrap(), "not a layout");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "not a layout",
            "the original is untouched"
        );
        // A file that cannot be copied is an error to report, not a panic.
        assert!(LayoutEngine::set_aside_unreadable(&dir.join("missing.ron"), now).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
