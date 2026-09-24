//! A session's active profile: the one policy file capsem-process enforces.
use super::*;

/// One active profile as written: where, and the digest of its exact bytes,
/// which capsem-process echoes back when it applies it.
pub(crate) struct PublishedActiveProfile {
    pub(crate) path: PathBuf,
    pub(crate) digest: String,
}

impl ServiceState {
    /// Write a session's active profile from `profile` and the corp config.
    pub(crate) fn materialize_active_profile(
        &self,
        profile: &Profile,
        session_dir: &StdPath,
    ) -> Result<PublishedActiveProfile> {
        let config = profile.config();
        let (_, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
        let plugins = self
            .plugin_policy_by_profile
            .lock()
            .unwrap()
            .get(&config.id)
            .cloned()
            .unwrap_or_default();
        let active_profile = ActiveProfileFile::from_profile_and_corp(profile, &corp, plugins)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("build active profile for {}", config.id))?;
        let active_profile_dir = session_dir.join(ACTIVE_PROFILE_DIR);
        std::fs::create_dir_all(&active_profile_dir)
            .with_context(|| format!("create {}", active_profile_dir.display()))?;
        let active_profile_path = active_profile_dir.join(ACTIVE_PROFILE_FILE);
        let serialized = toml::to_string_pretty(&active_profile).context("serialize active profile")?;
        // capsem-process reads this on every reload: publish it whole.
        capsem_foundation::unix::fs::atomic_write_private(&active_profile_path, serialized.as_bytes())
            .with_context(|| format!("write {}", active_profile_path.display()))?;
        let digest = capsem_core::net::policy_config::active_profile_digest(serialized.as_bytes());

        let stale_runtime_config = session_dir.join("runtime-config");
        if stale_runtime_config.exists() {
            std::fs::remove_dir_all(&stale_runtime_config)
                .with_context(|| format!("remove stale {}", stale_runtime_config.display()))?;
        }

        Ok(PublishedActiveProfile {
            path: active_profile_path,
            digest,
        })
    }

    /// Re-materialize the active profile of every running VM on `profile_filter`
    /// (every VM when `None`) and return each VM's id with what it was given.
    pub(crate) fn refresh_active_profiles(&self, profile_filter: Option<&str>) -> Result<Vec<(String, String)>> {
        let targets = {
            let instances = self.instances.lock().unwrap();
            instances
                .iter()
                .filter(|(_, info)| {
                    profile_filter
                        .map(|profile_id| info.profile_id == profile_id)
                        .unwrap_or(true)
                })
                .map(|(id, info)| (id.clone(), info.profile_id.clone(), info.session_dir.clone()))
                .collect::<Vec<_>>()
        };

        let mut published = Vec::with_capacity(targets.len());
        for (id, profile_id, session_dir) in &targets {
            let runtime_profile = self
                .profile_for_runtime(profile_id)
                .with_context(|| format!("load runtime profile {profile_id} for {id}"))?;
            let active = self
                .materialize_active_profile(&runtime_profile, session_dir)
                .with_context(|| {
                    format!(
                        "refresh active profile config for {id} ({profile_id}) in {}",
                        session_dir.display()
                    )
                })?;
            published.push((id.clone(), active.digest));
        }

        Ok(published)
    }
}
