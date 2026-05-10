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
mod tests;
