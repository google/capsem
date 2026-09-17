//! Session directory housekeeping: preserving crash evidence, culling old
//! failed sessions, and the contained delete every purge goes through.

use super::*;

/// `vm.resources.retention_days` when nothing resolves one. Matches the
/// setting's declared default, which is the number a user reading the settings
/// UI is being promised.
pub(crate) const DEFAULT_RETENTION_DAYS: u64 = 30;

/// How long a failed session's evidence is kept, from the user's settings.
///
/// Split from `retention_days_from_resolved` the way `automatic_updates_enabled`
/// is: the pure half is what tests exercise, so a test never depends on the
/// settings file of whoever is running it.
pub(crate) fn retention_days() -> u64 {
    let (user, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    retention_days_from_resolved(&capsem_core::net::policy_config::resolve_settings(&user, &corp))
}

pub(crate) fn retention_days_from_resolved(settings: &[capsem_core::net::policy_config::ResolvedSetting]) -> u64 {
    settings
        .iter()
        .find(|setting| setting.id == "vm.resources.retention_days")
        .and_then(|setting| setting.effective_value.as_number())
        // The setting's own floor is 1. A zero or negative value would mean
        // "delete evidence the moment it is written", which no one can have
        // meant by a retention period, so it falls back to the default.
        .filter(|days| *days >= 1)
        .map_or(DEFAULT_RETENTION_DAYS, |days| days as u64)
}

impl ServiceState {
    /// Rename an ephemeral session dir to a `-failed-*` sibling so its
    /// logs survive for post-mortem, then cull down to
    /// `MAX_FAILED_SESSIONS`.
    ///
    /// Three loss paths converge here: (a) `handle_run`'s
    /// `wait_for_vm_ready` timeout, (b) `scrub_evicted_instance` when
    /// cleanup detects a dead capsem-process, (c) the unexpected
    /// child-exit handler in `provision_sandbox`. All three cases are
    /// "the process we wanted died" -- exactly when you need
    /// `process.log`, `mcp-aggregator.stderr.log`, `serial.log`, and
    /// `session.db` most. Call this instead of `remove_dir_all` on
    /// every such path.
    ///
    /// If the rename fails (EEXIST, permission, different filesystem,
    /// etc.) we `warn!` with the specific error and fall back to
    /// `remove_dir_all` so disk isn't leaked when the filesystem is
    /// already unhappy.
    pub(crate) fn preserve_failed_session_dir(&self, session_dir: &std::path::Path, id: &str) -> Option<PathBuf> {
        let failed_id = format!("{}-failed-{}", id, capsem_core::session::generate_session_id(),);
        let failed_dir = self.run_dir.join("sessions").join(&failed_id);
        match std::fs::rename(session_dir, &failed_dir) {
            Ok(()) => {
                info!(
                    id,
                    path = %failed_dir.display(),
                    "preserved failed session dir for post-mortem"
                );
                if let Err(e) = self.cull_failed_sessions().map(|_| ()) {
                    warn!(
                        error = %e,
                        "failed to cull old failed session dirs -- disk may grow beyond {MAX_FAILED_SESSIONS}"
                    );
                }
                Some(failed_dir)
            }
            Err(e) => {
                warn!(
                    id,
                    from = %session_dir.display(),
                    to = %failed_dir.display(),
                    error = %e,
                    "failed to preserve session dir for post-mortem -- logs lost; removing to reclaim disk"
                );
                if let Err(e) = std::fs::remove_dir_all(session_dir) {
                    warn!(
                        id,
                        path = %session_dir.display(),
                        error = %e,
                        "also failed to remove session dir -- orphaned on disk"
                    );
                }
                None
            }
        }
    }

    /// Cull failed session dirs by age and by count, and say how many went.
    ///
    /// Reads `vm.resources.retention_days`; see
    /// `cull_failed_sessions_older_than` for what the two rules are for.
    pub(crate) fn cull_failed_sessions(&self) -> Result<usize> {
        self.cull_failed_sessions_older_than(retention_days())
    }

    /// Two rules, because they answer different questions.
    ///
    /// The count cap bounds disk: whatever happens, at most
    /// `MAX_FAILED_SESSIONS` post-mortems are kept. It says nothing about how
    /// long the newest 32 live, so a machine that fails once a month kept a
    /// session's bodies, logs and ledger for two and a half years.
    ///
    /// The age rule is the promise the settings UI makes. A user who sets a
    /// retention period is saying how long evidence about their work may sit
    /// on disk, and a directory that outlives it is a promise broken, whether
    /// or not anything is above the cap.
    pub(crate) fn cull_failed_sessions_older_than(&self, retention_days: u64) -> Result<usize> {
        let sessions_dir = self.run_dir.join("sessions");
        if !sessions_dir.exists() {
            return Ok(0);
        }
        let mut failed_dirs: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        let entries =
            std::fs::read_dir(&sessions_dir).with_context(|| format!("read_dir({})", sessions_dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.contains("-failed-") {
                continue;
            }
            // If we can't stat, skip rather than fail the whole cull --
            // we'd rather leave one undateable dir than abort the prune.
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    failed_dirs.push((path, modified));
                }
            }
        }
        failed_dirs.sort_by(|a, b| a.1.cmp(&b.1));
        let expired_before = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(retention_days.saturating_mul(86_400)))
            // A retention period so long it leaves the epoch behind expires
            // nothing, which is the honest reading of "keep it that long".
            .unwrap_or(std::time::UNIX_EPOCH);
        let expired = failed_dirs
            .iter()
            .take_while(|(_, modified)| *modified < expired_before)
            .count();
        let over_cap = failed_dirs.len().saturating_sub(MAX_FAILED_SESSIONS);
        // Oldest first, so both rules select a prefix of the same sorted list
        // and the wider one subsumes the other.
        let to_delete = expired.max(over_cap);
        let mut culled = 0;
        for (path, _) in failed_dirs.iter().take(to_delete) {
            info!(path = %path.display(), "culling old failed session dir");
            match std::fs::remove_dir_all(path) {
                Ok(()) => culled += 1,
                Err(e) => warn!(path = %path.display(), error = %e, "cull remove_dir_all failed"),
            }
        }
        Ok(culled)
    }

    /// Permanently remove one service-owned session directory.
    ///
    /// Persistent registry data is user-writable state, so never pass its
    /// `session_dir` directly to `remove_dir_all`. Restrict deletion to a
    /// real, direct child of this service's sessions/ or persistent/ roots
    /// and reject symlinks before performing the recursive removal.
    pub(crate) fn delete_session_dir(&self, session_dir: &StdPath) -> Result<()> {
        let parent = session_dir.parent().ok_or_else(|| {
            anyhow!(
                "refusing to delete session path without a parent: {}",
                session_dir.display()
            )
        })?;
        let allowed_parents = [self.run_dir.join("sessions"), self.run_dir.join("persistent")];

        let canonical_run_dir = self.run_dir.canonicalize().with_context(|| {
            format!(
                "canonicalize service run directory before delete: {}",
                self.run_dir.display()
            )
        })?;
        let canonical_requested_parent = parent.canonicalize().with_context(|| {
            format!(
                "canonicalize requested session root before delete: {}",
                parent.display()
            )
        })?;
        let mut canonical_parent = None;
        for allowed_parent in &allowed_parents {
            let parent_metadata = match std::fs::symlink_metadata(allowed_parent) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "inspect service session root before delete: {}",
                            allowed_parent.display()
                        )
                    });
                }
            };
            if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
                if parent == allowed_parent.as_path() {
                    return Err(anyhow!(
                        "refusing to delete through non-directory service session root: {}",
                        allowed_parent.display()
                    ));
                }
                continue;
            }

            let candidate = allowed_parent.canonicalize().with_context(|| {
                format!(
                    "canonicalize service session root before delete: {}",
                    allowed_parent.display()
                )
            })?;
            if candidate.parent() != Some(canonical_run_dir.as_path()) {
                if canonical_requested_parent == candidate {
                    return Err(anyhow!(
                        "refusing to delete through session root outside canonical run directory: {}",
                        allowed_parent.display()
                    ));
                }
                continue;
            }
            if canonical_requested_parent == candidate {
                canonical_parent = Some(candidate);
                break;
            }
        }
        let canonical_parent = canonical_parent.ok_or_else(|| {
            anyhow!(
                "refusing to delete session path outside service roots: {}",
                session_dir.display()
            )
        })?;

        let metadata = match std::fs::symlink_metadata(session_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect session path before delete: {}", session_dir.display()));
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(anyhow!(
                "refusing to recursively delete non-directory session path: {}",
                session_dir.display()
            ));
        }

        let canonical_session = session_dir.canonicalize().with_context(|| {
            format!(
                "canonicalize service session path before delete: {}",
                session_dir.display()
            )
        })?;
        if canonical_session.parent() != Some(canonical_parent.as_path()) {
            return Err(anyhow!(
                "refusing to delete session path outside canonical service root: {}",
                session_dir.display()
            ));
        }

        // Remove the already verified canonical child, not a registry-provided
        // alias. This keeps a legitimate macOS /var -> /private/var spelling
        // difference working without giving a mutable alias another path
        // resolution opportunity at the destructive operation.
        remove_quiesced_session_dir(&canonical_session)
            .with_context(|| format!("delete canonical session directory: {}", canonical_session.display()))
    }

    pub(crate) fn reconcile_persistent_defunct_from_logs(&self) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or_default();
        let last_ms = self.last_defunct_reconcile_ms.load(Ordering::Acquire);
        if now_ms.saturating_sub(last_ms) < 1_000 {
            return;
        }
        if self
            .last_defunct_reconcile_ms
            .compare_exchange(last_ms, now_ms, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let candidates: Vec<(String, PathBuf)> = {
            let registry = self.persistent_registry.lock().unwrap();
            let instances = self.instances.lock().unwrap();
            registry
                .list()
                .filter(|entry| !entry.defunct)
                .filter(|entry| !instances.contains_key(&persistent_entry_vm_id(entry)))
                .map(|entry| (entry.name.clone(), entry.session_dir.clone()))
                .collect()
        };

        let updates: Vec<(String, String)> = candidates
            .into_iter()
            .filter_map(|(name, session_dir)| read_boot_failure_tail(&session_dir).map(|tail| (name, tail)))
            .collect();
        if updates.is_empty() {
            return;
        }

        let mut registry = self.persistent_registry.lock().unwrap();
        let instances = self.instances.lock().unwrap();
        let mut changed = false;
        for (name, tail) in updates {
            if instances.contains_key(&name) {
                continue;
            }
            if let Some(entry) = registry.get_mut(&name) {
                if !entry.defunct {
                    warn!(
                        name,
                        cause = capsem_core::session::boot_failure_summary(&tail),
                        "marking persistent VM defunct from preserved boot logs"
                    );
                    entry.defunct = true;
                    entry.last_error = Some(tail);
                    entry.suspended = false;
                    entry.checkpoint_path = None;
                    changed = true;
                }
            }
        }
        drop(instances);
        if changed {
            if let Err(error) = registry.save() {
                error!(error = %error, "failed to save persistent registry after defunct reconciliation");
            }
        }
    }
}
