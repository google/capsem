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
    /// Whether the GUI onboarding wizard has been completed.
    /// Non-interactive CLI setup leaves this false; the app wizard sets it true.
    #[serde(default)]
    pub onboarding_completed: bool,
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
}

/// Load setup state from a JSON file. Returns default if missing or invalid.
pub fn load_state(path: &Path) -> SetupState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
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

/// Default path to setup-state.json: ~/.capsem/setup-state.json
pub fn default_state_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(".capsem").join("setup-state.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_file_returns_default() {
        let state = load_state(Path::new("/nonexistent/setup-state.json"));
        assert_eq!(state.schema_version, 0);
        assert!(!state.onboarding_completed);
        assert!(state.completed_steps.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup-state.json");

        let mut state = SetupState::default();
        state.schema_version = 2;
        state.mark_done("welcome");
        state.mark_done("providers");
        state.security_preset = Some("medium".to_string());
        state.onboarding_completed = true;

        save_state(&path, &state).unwrap();
        let loaded = load_state(&path);

        assert_eq!(loaded.schema_version, 2);
        assert!(loaded.is_step_done("welcome"));
        assert!(loaded.is_step_done("providers"));
        assert!(!loaded.is_step_done("summary"));
        assert_eq!(loaded.security_preset.as_deref(), Some("medium"));
        assert!(loaded.onboarding_completed);
    }

    #[test]
    fn backward_compat_missing_onboarding_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup-state.json");

        // Write a v1 state file without onboarding_completed
        let json = r#"{"schema_version":1,"completed_steps":["welcome"],"security_preset":"medium","providers_done":true,"repositories_done":true,"service_installed":true,"vm_verified":false,"corp_config_source":null}"#;
        std::fs::write(&path, json).unwrap();

        let loaded = load_state(&path);
        assert_eq!(loaded.schema_version, 1);
        assert!(!loaded.onboarding_completed); // defaults to false
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
