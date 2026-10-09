//! A session's active policy: the one policy file capsem-process enforces.
//!
//! It is built from the built-in defaults, the user's settings.toml and the
//! corp config, written into the session, and pushed to running VMs whenever
//! one of its inputs is edited through the service.
use super::*;

use capsem_core::net::policy_config::ActivePolicyFile;

/// One active policy as written: where, and the digest of its exact bytes,
/// which capsem-process echoes back when it applies it.
pub(crate) struct PublishedActivePolicy {
    pub(crate) path: PathBuf,
    pub(crate) digest: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) runtime: Arc<capsem_core::net::policy_config::CompiledActivePolicy>,
}

impl PublishedActivePolicy {
    pub(crate) fn broker_policy(&self) -> Arc<crate::upstream_broker::BrokerPolicy> {
        crate::upstream_broker::BrokerPolicy::new(self.digest.clone(), Arc::clone(&self.runtime))
    }
}

impl ServiceState {
    /// Write a session's active policy from the current settings and corp files.
    pub(crate) fn materialize_active_policy(&self, session_dir: &StdPath) -> Result<PublishedActivePolicy> {
        let (settings, corp) = capsem_core::net::policy_config::load_policy_files()
            .map_err(anyhow::Error::msg)
            .context("load the policy inputs")?;
        let active_policy = ActivePolicyFile::from_settings_and_corp(&settings, &corp)
            .map_err(anyhow::Error::msg)
            .context("build the active policy")?;
        let active_policy_dir = session_dir.join(ACTIVE_POLICY_DIR);
        std::fs::create_dir_all(&active_policy_dir)
            .with_context(|| format!("create {}", active_policy_dir.display()))?;
        let active_policy_path = active_policy_dir.join(ACTIVE_POLICY_FILE);
        let serialized = toml::to_string_pretty(&active_policy).context("serialize active policy")?;
        let exact: ActivePolicyFile = toml::from_str(&serialized).context("parse serialized active policy")?;
        let runtime = Arc::new(
            exact
                .compile_runtime()
                .map_err(anyhow::Error::msg)
                .context("compile serialized active policy")?,
        );
        // Keep the durable audit copy whole; the same exact bytes are sent to
        // capsem-process over its authenticated coordinator channel.
        capsem_foundation::unix::fs::atomic_write_private(&active_policy_path, serialized.as_bytes())
            .with_context(|| format!("write {}", active_policy_path.display()))?;
        Ok(PublishedActivePolicy {
            path: active_policy_path,
            digest: capsem_core::net::policy_config::active_policy_digest(serialized.as_bytes()),
            bytes: serialized.into_bytes(),
            runtime,
        })
    }

    /// Re-materialize the active policy of every running VM. Returns each VM's
    /// id with the digest it was given, or why it could not be; one VM that
    /// cannot take the policy does not keep the others on the old one.
    pub(crate) fn refresh_active_policies(&self) -> Vec<(String, std::result::Result<PublishedActivePolicy, String>)> {
        // Copied out first: the instances lock is not held across file writes.
        let mut targets = Vec::new();
        for (id, info) in self.instances.lock().unwrap().iter() {
            targets.push((id.clone(), info.session_dir.clone()));
        }
        targets
            .into_iter()
            .map(|(id, session_dir)| {
                let published = self
                    .materialize_active_policy(&session_dir)
                    .map_err(|error| format!("{error:#}"));
                (id, published)
            })
            .collect()
    }
}

/// Deliver the current policy to every running VM: re-materialize each
/// session's active policy, then send those exact bytes to capsem-process. Mutation
/// routes call this so an edit is enforced before the route returns.
///
/// A VM counts as updated only when it reports applying the exact bytes this
/// call wrote; an acknowledgement of some other active policy -- one written
/// by a concurrent edit -- is a failure. One VM that fails does not stop the
/// others: the edit is applied everywhere it can be, and the error names the
/// VMs left on their previous policy. A VM silently left on stale policy is
/// the worse outcome, so any such VM fails the request.
pub(crate) async fn push_policy_to_running_instances(
    state: &Arc<ServiceState>,
    _mutation: &PolicyMutation<'_>,
) -> Result<usize, AppError> {
    let published = state.off_worker(|state| state.refresh_active_policies()).await?;
    let mut total = published.len();

    let mut failures = Vec::new();
    let targets = {
        let instances = state.instances.lock().unwrap();
        published
            .into_iter()
            .filter_map(|(id, digest)| match digest {
                Err(error) => {
                    failures.push(format!("{id}: {error}"));
                    None
                }
                // A VM that stopped since it was listed has nothing to reload.
                Ok(published) => {
                    let instance = instances.get(&id)?;
                    Some((id.clone(), instance.generation, instance.uds_path.clone(), published))
                }
            })
            .collect::<Vec<_>>()
    };

    let results = futures::future::join_all(targets.iter().map(|(id, generation, uds_path, published)| async move {
        let expected = &published.digest;
        let request = ServiceToProcess::ReloadConfig {
            id: state.next_job_id(),
            active_policy: published.bytes.clone(),
        };
        match send_ipc_command(state, uds_path, request, Some(5)).await {
            Ok(ProcessToService::ConfigReloadResult {
                active_policy_digest: Some(applied),
                error: None,
                ..
            }) if &applied == expected => {
                let publisher = {
                    let instances = state.instances.lock().unwrap();
                    match instances.get(id) {
                        Some(instance) if instance.generation == *generation => Some(instance.upstream_policy.clone()),
                        Some(_) => return Some(format!("{id}: owner changed before broker policy publication")),
                        None => None,
                    }
                };
                match publisher {
                    Some(publisher) => {
                        let proxy = match state.proxy_worker(id, *generation) {
                            Ok(proxy) => proxy,
                            Err(error) => return Some(format!("{id}: {error}")),
                        };
                        match proxy.apply_policy(published.bytes.clone()).await {
                            Ok(applied) if applied == *expected => publisher
                                .publish(published.broker_policy())
                                .await
                                .err()
                                .map(|error| format!("{id}: {error}")),
                            Ok(applied) => Some(format!(
                                "{id}: proxy applied active policy {applied}, expected {expected}"
                            )),
                            Err(error) => Some(format!("{id}: proxy reload refused: {error:#}")),
                        }
                    }
                    None => None,
                }
            }
            Ok(ProcessToService::ConfigReloadResult {
                active_policy_digest: Some(applied),
                error: None,
                ..
            }) => Some(format!("{id}: applied active policy {applied}, expected {expected}")),
            Ok(ProcessToService::ConfigReloadResult { error: Some(error), .. }) => {
                Some(format!("{id}: reload refused: {error}"))
            }
            Ok(_) => Some(format!("{id}: unexpected response")),
            Err(e) => Some(format!("{id}: {e}")),
        }
    }))
    .await;
    failures.extend(results.into_iter().flatten());
    let (standalone_total, standalone_failures) = crate::standalone_proxy::refresh_policies(state).await?;
    total += standalone_total;
    failures.extend(standalone_failures);

    if failures.is_empty() {
        Ok(total)
    } else {
        let subjects = if standalone_total == 0 {
            "VMs"
        } else {
            "VMs and proxy sessions"
        };
        Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "policy applied to {} of {total} running {subjects}; not applied: {}",
                total - failures.len(),
                failures.join(", ")
            ),
        ))
    }
}
