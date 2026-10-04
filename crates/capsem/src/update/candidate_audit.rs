use super::*;

/// The audit trail of one fetched release candidate: every event names the
/// exact manifest bytes and the installed state they would replace.
pub(super) struct CandidateAudit<'a> {
    source: &'a str,
    manifest_sha256: String,
    installed_assets: PathBuf,
    previous: serde_json::Value,
}

impl<'a> CandidateAudit<'a> {
    pub(super) fn new(source: &'a str, manifest_bytes: &[u8], installed_assets: PathBuf) -> Self {
        let previous = installed_asset_audit_state(&installed_assets);
        Self {
            source,
            manifest_sha256: sha256_hex(manifest_bytes),
            installed_assets,
            previous,
        }
    }

    pub(super) fn installed_assets(&self) -> &Path {
        &self.installed_assets
    }

    pub(super) fn fetched(&self) {
        self.record("release_candidate_fetched", "fetched", None, None);
    }

    pub(super) fn rejected(&self, error: &anyhow::Error) {
        let current = installed_asset_audit_state(&self.installed_assets);
        self.record("release_candidate_rejected", "failure", Some(current), Some(error));
    }

    pub(super) fn activated(&self) {
        let current = installed_asset_audit_state(&self.installed_assets);
        self.record("release_candidate_activated", "success", Some(current), None);
    }

    fn record(&self, event: &str, outcome: &str, current: Option<serde_json::Value>, error: Option<&anyhow::Error>) {
        let mut entry = serde_json::json!({
            "event": event,
            "action": "release_candidate",
            "outcome": outcome,
            "source": self.source,
            "channel": channel_from_source(self.source),
            "candidate_manifest_sha256": self.manifest_sha256,
            "previous": self.previous,
        });
        if let Some(current) = current {
            entry["current"] = current;
        }
        if let Some(error) = error {
            entry["error"] = serde_json::Value::String(format!("{error:#}"));
        }
        append_update_audit(entry);
    }
}
