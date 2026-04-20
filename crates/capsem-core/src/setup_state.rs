//! Setup state persistence for the onboarding wizard.
//!
//! `setup-state.json` lives at `~/.capsem/setup-state.json` and tracks which
//! setup steps have been completed, the chosen security preset, and whether
//! the GUI onboarding wizard has been finished.
//!
//! Shared between the CLI (`capsem setup`) and the service (setup API
//! endpoints).

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::warn;

/// Current schema version for the GUI onboarding wizard. Bump when the wizard
/// gains new steps or a UX overhaul that existing users should see again. On
/// next launch, any state whose `onboarding_version` is below this value will
/// re-trigger the wizard. Separate from the CLI install flow -- the install
/// itself is gated by `install_completed`.
pub const CURRENT_ONBOARDING_VERSION: u32 = 1;

/// Persistent state written to ~/.capsem/setup-state.json.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SetupState {
    pub schema_version: u32,
    #[serde(default)]
    pub completed_steps: Vec<String>,
    pub security_preset: Option<String>,
    #[serde(default)]
    pub providers_done: bool,
    #[serde(default)]
    pub repositories_done: bool,
    #[serde(default)]
    pub service_installed: bool,
    #[serde(default)]
    pub vm_verified: bool,
    pub corp_config_source: Option<String>,
    /// Whether `capsem setup` finished its mandatory steps (CLI install flow).
    /// Separate from `onboarding_completed` -- the CLI sets this true on success
    /// regardless of whether the user has seen the GUI wizard.
    #[serde(default)]
    pub install_completed: bool,
    /// Whether the GUI onboarding wizard has been completed.
    /// Non-interactive CLI setup leaves this false; the app wizard sets it true.
    #[serde(default)]
    pub onboarding_completed: bool,
    /// Which version of the GUI onboarding wizard the user last completed. Paired
    /// with `CURRENT_ONBOARDING_VERSION` to force re-onboarding on release.
    #[serde(default)]
    pub onboarding_version: u32,
}

impl SetupState {
    pub fn is_step_done(&self, step: &str) -> bool {
        self.completed_steps.iter().any(|s| s == step)
    }

    pub fn mark_done(&mut self, step: &str) {
        if !self.is_step_done(step) {
            self.completed_steps.push(step.to_string());
        }
    }

    /// Has the user completed the current GUI onboarding wizard version?
    /// False if they never finished it OR if we've since bumped the wizard
    /// version (e.g. a release with a new wizard step).
    pub fn needs_onboarding(&self) -> bool {
        !self.onboarding_completed || self.onboarding_version < CURRENT_ONBOARDING_VERSION
    }

    /// Reset only the GUI wizard flags; leave install state intact. Used by
    /// `capsem setup --force-onboarding` and release upgrades.
    pub fn reset_onboarding(&mut self) {
        self.onboarding_completed = false;
        self.onboarding_version = 0;
    }
}

/// Load setup state from a JSON file. Returns default if the file is missing
/// or unreadable; also returns default (with a warning log) if the file exists
/// but fails to parse -- a corrupt state file silently resetting the user's
/// progress is worse than surfacing the problem via logs.
pub fn load_state(path: &Path) -> SetupState {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SetupState::default(),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to read setup-state.json; resetting to defaults");
            return SetupState::default();
        }
    };
    match serde_json::from_str::<SetupState>(&contents) {
        Ok(mut state) => {
            // Backward-compat: state files written before `install_completed`
            // existed have it default to false on load. If the setup flow
            // previously reached the summary step, the install was clearly
            // complete -- honor that so existing users don't see a spurious
            // "install didn't finish" banner after upgrading.
            if !state.install_completed && state.is_step_done("summary") {
                state.install_completed = true;
            }
            state
        }
        Err(e) => {
            warn!(
                path = %path.display(),
                error = %e,
                "setup-state.json is corrupt; resetting to defaults (setup will re-run all steps)",
            );
            SetupState::default()
        }
    }
}

/// Save setup state to a JSON file (atomic write via temp file).
pub fn save_state(path: &Path, state: &SetupState) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Default path to setup-state.json inside the capsem home dir.
pub fn default_state_path() -> Option<std::path::PathBuf> {
    crate::paths::capsem_home_opt().map(|h| h.join("setup-state.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_file_returns_default() {
        let state = load_state(Path::new("/nonexistent/setup-state.json"));
        assert_eq!(state.schema_version, 0);
        assert!(!state.onboarding_completed);
        assert!(!state.install_completed);
        assert_eq!(state.onboarding_version, 0);
        assert!(state.completed_steps.is_empty());
    }

    #[test]
    fn default_state_needs_onboarding() {
        let state = SetupState::default();
        assert!(state.needs_onboarding());
    }

    #[test]
    fn completed_current_version_does_not_need_onboarding() {
        let state = SetupState {
            onboarding_completed: true,
            onboarding_version: CURRENT_ONBOARDING_VERSION,
            ..SetupState::default()
        };
        assert!(!state.needs_onboarding());
    }

    #[test]
    fn older_onboarding_version_triggers_rewalk() {
        // User finished an older wizard version. A release bumped the version.
        // They should see the wizard again.
        let state = SetupState {
            onboarding_completed: true,
            onboarding_version: 0,
            ..SetupState::default()
        };
        if CURRENT_ONBOARDING_VERSION > 0 {
            assert!(state.needs_onboarding());
        }
    }

    #[test]
    fn reset_onboarding_preserves_install_state() {
        let mut state = SetupState {
            install_completed: true,
            onboarding_completed: true,
            onboarding_version: CURRENT_ONBOARDING_VERSION,
            security_preset: Some("medium".into()),
            ..SetupState::default()
        };
        state.mark_done("summary");
        state.reset_onboarding();
        assert!(!state.onboarding_completed);
        assert_eq!(state.onboarding_version, 0);
        assert!(state.install_completed, "install state must survive a wizard reset");
        assert!(state.is_step_done("summary"));
        assert_eq!(state.security_preset.as_deref(), Some("medium"));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup-state.json");

        let mut state = SetupState {
            schema_version: 2,
            install_completed: true,
            onboarding_completed: true,
            onboarding_version: CURRENT_ONBOARDING_VERSION,
            ..SetupState::default()
        };
        state.mark_done("welcome");
        state.mark_done("providers");
        state.security_preset = Some("medium".to_string());

        save_state(&path, &state).unwrap();
        let loaded = load_state(&path);

        assert_eq!(loaded.schema_version, 2);
        assert!(loaded.is_step_done("welcome"));
        assert!(loaded.is_step_done("providers"));
        assert!(!loaded.is_step_done("summary"));
        assert_eq!(loaded.security_preset.as_deref(), Some("medium"));
        assert!(loaded.install_completed);
        assert!(loaded.onboarding_completed);
        assert_eq!(loaded.onboarding_version, CURRENT_ONBOARDING_VERSION);
    }

    #[test]
    fn load_state_returns_default_on_corrupt_json() {
        // A corrupt state file must not panic and must not propagate the parse
        // error; it should return Default and emit a warn-level log (not
        // asserted here, but pinned in the function's doc comment).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup-state.json");
        std::fs::write(&path, b"{ this is not valid json").unwrap();

        let loaded = load_state(&path);
        assert_eq!(loaded.schema_version, 0);
        assert!(loaded.completed_steps.is_empty());
        assert!(loaded.security_preset.is_none());
    }

    #[test]
    fn load_state_returns_default_on_non_object_json() {
        // Valid JSON but wrong shape (array instead of object) should also be
        // treated as corrupt and reset -- not silently accepted as empty.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup-state.json");
        std::fs::write(&path, b"[]").unwrap();

        let loaded = load_state(&path);
        assert_eq!(loaded.schema_version, 0);
    }

    #[test]
    fn backward_compat_infers_install_completed_from_summary_step() {
        // A pre-upgrade state file will not have `install_completed`. If the
        // summary step was reached, load_state should infer install=done so
        // the UI doesn't warn "install didn't finish" on upgrade.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup-state.json");
        let json = r#"{"schema_version":2,"completed_steps":["welcome","security_preset","providers","repositories","summary"],"security_preset":"medium","providers_done":true,"repositories_done":true,"service_installed":true,"vm_verified":false,"corp_config_source":null,"onboarding_completed":true}"#;
        std::fs::write(&path, json).unwrap();

        let loaded = load_state(&path);
        assert!(loaded.install_completed, "pre-upgrade state with summary step must infer install_completed");
    }

    #[test]
    fn backward_compat_does_not_infer_install_completed_for_partial_setup() {
        // State file that didn't reach summary step -- install really is
        // incomplete, do not fabricate completeness.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup-state.json");
        let json = r#"{"schema_version":2,"completed_steps":["welcome"],"security_preset":null}"#;
        std::fs::write(&path, json).unwrap();

        let loaded = load_state(&path);
        assert!(!loaded.install_completed);
    }

    #[test]
    fn backward_compat_missing_onboarding_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup-state.json");

        // Write a v1 state file without onboarding_completed, install_completed,
        // or onboarding_version -- all three must default cleanly.
        let json = r#"{"schema_version":1,"completed_steps":["welcome"],"security_preset":"medium","providers_done":true,"repositories_done":true,"service_installed":true,"vm_verified":false,"corp_config_source":null}"#;
        std::fs::write(&path, json).unwrap();

        let loaded = load_state(&path);
        assert_eq!(loaded.schema_version, 1);
        assert!(!loaded.onboarding_completed);
        assert!(!loaded.install_completed);
        assert_eq!(loaded.onboarding_version, 0);
        assert!(loaded.is_step_done("welcome"));
    }

    #[test]
    fn mark_done_is_idempotent() {
        let mut state = SetupState::default();
        state.mark_done("test");
        state.mark_done("test");
        assert_eq!(state.completed_steps.len(), 1);
    }
}
