//! Session directory housekeeping: preserving crash evidence, culling old
//! failed sessions, and the contained delete every purge goes through.

use super::*;

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
                if let Err(e) = self.cull_failed_sessions() {
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

    pub(crate) fn cull_failed_sessions(&self) -> Result<()> {
        let sessions_dir = self.run_dir.join("sessions");
        if !sessions_dir.exists() {
            return Ok(());
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
        if failed_dirs.len() > MAX_FAILED_SESSIONS {
            let to_delete = failed_dirs.len() - MAX_FAILED_SESSIONS;
            for (path, _) in failed_dirs.iter().take(to_delete) {
                info!(path = %path.display(), "culling old failed session dir");
                if let Err(e) = std::fs::remove_dir_all(path) {
                    warn!(path = %path.display(), error = %e, "cull remove_dir_all failed");
                }
            }
        }
        Ok(())
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
}
