use super::*;

pub(crate) fn configure_resume_storage(command: &mut tokio::process::Command, entry: &PersistentVmEntry) -> Result<()> {
    command
        .arg("--scratch-disk-size-gb")
        .arg(session_rootfs_size_gb(entry)?.to_string());
    Ok(())
}

pub(crate) fn configure_resume_checkpoint(
    command: &mut tokio::process::Command,
    entry: &PersistentVmEntry,
    stage_refreshed: bool,
) -> bool {
    if stage_refreshed {
        info!(
            name = entry.name,
            "workload controller changed, cold booting instead of restoring its checkpoint"
        );
        return false;
    }
    if entry.suspended {
        if let Some(ref checkpoint) = entry.checkpoint_path {
            let full_checkpoint = entry.session_dir.join(checkpoint);
            let complete = checkpoint_complete_path(&full_checkpoint);
            if full_checkpoint.exists() && complete.exists() {
                command.arg("--checkpoint-path").arg(&full_checkpoint);
                info!(name = entry.name, checkpoint = %full_checkpoint.display(), "warm restore from checkpoint");
                return true;
            }
            tracing::warn!(
                name = entry.name,
                checkpoint = %full_checkpoint.display(),
                complete = %complete.display(),
                "checkpoint incomplete, cold booting"
            );
        }
    }
    false
}

impl ServiceState {
    /// Resume a stopped persistent VM by re-spawning capsem-process against its
    /// existing session directory.
    pub(crate) fn resume_sandbox(
        self: &Arc<Self>,
        id: &str,
        ram_mb_override: Option<u64>,
        cpus_override: Option<u32>,
    ) -> Result<String> {
        let _launch = self.lifecycle.admit()?;
        self.cleanup_stale_instances();
        self.reconcile_persistent_defunct_from_logs();

        let entry = find_persistent_entry_by_route_id(self.as_ref(), id)
            .ok_or_else(|| anyhow!("no persistent VM with id \"{}\"", id))?;
        let vm_id = persistent_entry_vm_id(&entry);
        if vm_id != id {
            return Err(anyhow!(
                "route id mismatch: requested \"{}\", registry has \"{}\"",
                id,
                vm_id
            ));
        }
        let name = entry.name.clone();

        // Check if already running
        {
            let instances = self.instances.lock().unwrap();
            if instances.contains_key(&vm_id) {
                return Ok(vm_id);
            }
        }

        // The previous process's reaper may not have moved a session that was
        // persisted while running home yet; settle it here so this launch
        // starts from the directory it will keep.
        let mut entry = entry;
        entry.session_dir = vm_lifecycle::settle_persistent_session_dir(self, &entry.name, &entry.session_dir);
        if !entry.session_dir.exists() {
            return Err(anyhow!("session directory for \"{}\" is missing", name));
        }
        if entry.defunct {
            let reason = entry
                .last_error
                .as_deref()
                .unwrap_or("previous boot failed before the VM reached ready");
            return Err(anyhow!("persistent VM \"{}\" is defunct: {}", name, reason));
        }

        let ram_mb = ram_mb_override.unwrap_or(entry.ram_mb);
        let cpus = cpus_override.unwrap_or(entry.cpus);
        let version = entry.base_version.clone();

        info!(name, version, "resume_sandbox: re-spawning process");

        let uds_path = self.instance_socket_path(&vm_id)?;
        let _ = std::fs::create_dir_all(uds_path.parent().unwrap());

        // Clear stale UDS + ready sentinel from the prior boot. Without this,
        // wait_for_vm_ready returns instantly against the old .ready file and
        // callers race ahead before the resumed agent has reconnected.
        let _ = std::fs::remove_file(&uds_path);
        service_runtime::remove_instance_sentinels(&uds_path);

        self.validate_persistent_entry(&entry)?;
        let active_policy = self.materialize_active_policy(&entry.session_dir)?;
        let active_policy_path = active_policy.path.clone();
        let (upstream_broker, owner_upstream_policy) = crate::upstream_broker::PendingBroker::pair_for_session(
            active_policy.broker_policy(),
            entry.session_dir.clone(),
        )?;
        let upstream_broker =
            upstream_broker.with_vm_ledger(Arc::clone(&self.ledger_workers), &vm_id, &entry.session_dir);
        let upstream_stdio = upstream_broker.worker_stdio()?;
        let (proxy_upstream_broker, proxy_upstream_policy) =
            crate::upstream_broker::PendingBroker::pair(active_policy.broker_policy())?;
        let proxy_upstream = proxy_upstream_broker.worker_stream()?;
        let upstream_policy = owner_upstream_policy.merge(proxy_upstream_policy);
        let resolved = self.resolve_pinned_asset_paths(&entry.asset_pins)?;
        self.validate_pinned_asset_files(&resolved, &entry.asset_pins)?;

        let process_log_path = entry.session_dir.join("process.log");
        let process_log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&process_log_path)
            .context("failed to open process.log")?;

        let mut child_cmd = tokio::process::Command::new(&self.process_binary);
        if !self.process_binary.exists() {
            child_cmd = tokio::process::Command::new("cache/target/cargo/debug/capsem-process");
        }
        configure_resume_storage(&mut child_cmd, &entry)?;

        // Inject VM identity so the guest knows its own name/ID.
        guest_env::stage(&entry.session_dir, &vm_id, &name, entry.env.as_ref())?;
        child_cmd.arg("--vm-name").arg(&name);
        crate::service_runtime::telemetry_export::grant_metric_broker(
            &mut child_cmd,
            &capsem_core::net::policy_config::load_settings_and_corp_files().1,
        );

        // Refresh a legacy staged controller before deciding whether its old
        // VM memory is safe to restore. A changed ownership map needs one cold
        // boot; current stages keep the warm path.
        let stage_refreshed = crate::container_setup::refresh_staged_workspace_owner(&entry.session_dir)
            .map_err(|error| anyhow!(error))?;
        let warm = configure_resume_checkpoint(&mut child_cmd, &entry, stage_refreshed);
        // A cold boot relaunches the workload; the last boot's markers would
        // report it running (or exited) before this boot's launcher starts. A
        // warm restore resumes that same workload, so its markers stay true.
        if !warm {
            crate::container_setup::forget_previous_run(&entry.session_dir).map_err(|e| anyhow!(e))?;
        }

        // Clear inherited env to prevent API key/token leakage, then
        // re-add only the minimal set needed for the process to function.
        // CAPSEM_HOME and CAPSEM_CORP_CONFIG are forwarded so the child loads
        // the same settings/corp contract as the service.
        child_cmd.env_clear();
        for key in PROCESS_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
            }
        }
        // W4: propagate trace context (resume path).
        for (k, v) in capsem_foundation::telemetry::child_trace_env(&vm_id) {
            child_cmd.env(k, v);
        }

        let generation = capsem_crypto::uuid_v4();
        crate::instance::persist_spawn_identity(&entry.session_dir, &vm_id, generation)?;
        let process_spawn_span = tracing::debug_span!(
            target: "capsem.launch",
            capsem_foundation::telemetry::LAUNCH_PROCESS_SPAWN_SPAN,
            boot_mode = "resume",
            status = tracing::field::Empty,
        );
        let (child, retirement) = match process_spawn_span.in_scope(|| {
            child_cmd
                .env(
                    "RUST_LOG",
                    std::env::var("RUST_LOG")
                        .unwrap_or_else(|_| capsem_foundation::telemetry::with_subsys_targets("capsem=info")),
                )
                .arg("--id")
                .arg(&vm_id)
                .arg("--parent-pid")
                .arg(std::process::id().to_string())
                .arg("--spawn-generation")
                .arg(generation.to_string())
                .arg("--assets-dir")
                .arg(&self.assets_dir)
                .arg("--rootfs")
                .arg(&resolved.rootfs)
                .arg("--kernel")
                .arg(&resolved.kernel)
                .arg("--initrd")
                .arg(&resolved.initrd)
                // The VM's own pins. Boot verifies against these, never
                // against whatever the installed manifest names today.
                .arg("--expected-kernel-hash")
                .arg(&entry.asset_pins.kernel.hash)
                .arg("--expected-initrd-hash")
                .arg(&entry.asset_pins.initrd.hash)
                .arg("--expected-rootfs-hash")
                .arg(&entry.asset_pins.rootfs.hash)
                .arg("--session-dir")
                .arg(&entry.session_dir)
                .arg("--active-policy")
                .arg(&active_policy_path)
                .arg("--retention-days")
                .arg(crate::session_housekeeping::retention_days().to_string())
                .arg("--cpus")
                .arg(cpus.to_string())
                .arg("--ram-mb")
                .arg(ram_mb.to_string())
                .arg("--uds-path")
                .arg(&uds_path)
                // Explicitly, because `uds_path` may have been shortened out
                // of the run tree and cannot be walked back up.
                .arg("--run-dir")
                .arg(&self.run_dir)
                .arg("--service-socket")
                .arg(&self.service_socket)
                .stdin(upstream_stdio)
                .stdout(std::process::Stdio::from(process_log_file.try_clone()?))
                .stderr(std::process::Stdio::from(process_log_file));
            self.retirements.spawn(&vm_id, generation, &mut child_cmd)
        }) {
            Ok(child) => {
                process_spawn_span.record("status", "ok");
                child
            }
            Err(error) => {
                process_spawn_span.record("status", "error");
                return Err(error.context("failed to spawn capsem-process"));
            }
        };

        let pid = child.id().unwrap_or(0);
        info!(name, pid, "capsem-process resumed");

        let proxy = match crate::proxy_worker::spawn_for_session(
            &self.proxy_binary,
            active_policy.bytes,
            &entry.session_dir.join("proxy.log"),
        ) {
            Ok(proxy) => proxy,
            Err(error) => {
                let _reaper = instance_reaper::kill_and_reap(child, retirement);
                return Err(error.context("start confined proxy worker"));
            }
        };
        if let Err(error) =
            tokio::runtime::Handle::current().block_on(self.grant_proxy_ledger(&vm_id, &entry.session_dir, &proxy))
        {
            let _reaper = instance_reaper::kill_and_reap(child, retirement);
            return Err(error);
        }
        if let Err(error) = tokio::runtime::Handle::current().block_on(self.grant_proxy_credentials(&proxy)) {
            let _reaper = instance_reaper::kill_and_reap(child, retirement);
            return Err(error);
        }
        if let Err(error) = tokio::runtime::Handle::current().block_on(self.grant_proxy_private_names(&vm_id, &proxy)) {
            let _reaper = instance_reaper::kill_and_reap(child, retirement);
            return Err(error);
        }
        if let Err(error) = tokio::runtime::Handle::current().block_on(proxy.grant(
            capsem_proto::proxy_control::ProxyCapability::Upstream,
            proxy_upstream.into(),
        )) {
            let _reaper = instance_reaper::kill_and_reap(child, retirement);
            return Err(error.context("grant proxy upstream broker"));
        }
        let upstream_broker = upstream_broker.with_proxy(proxy.clone());

        info!(
            vm_id,
            operation = "defer_session_db_handle_registration",
            session_dir = %entry.session_dir.display(),
            "session ledger reader will be granted lazily to the first route"
        );

        let authority = crate::instance::WorkerAuthority::default();
        let upstream_grant = authority.grant();
        let proxy_upstream_grant = authority.grant();
        let session_dir = entry.session_dir.clone();
        let mut instances = self.instances.lock().unwrap();
        instances.insert(
            vm_id.clone(),
            InstanceInfo {
                generation,
                authority,
                upstream_policy,
                id: vm_id.clone(),
                name: entry.name.clone(),
                asset_pins: entry.asset_pins.clone(),
                pid,
                uds_path: uds_path.clone(),
                session_dir: session_dir.clone(),
                ram_mb,
                cpus,
                start_time: std::time::Instant::now(),
                base_version: version,
                persistent: true,
                env: None,
                labels: entry.labels,
                forked_from: entry.forked_from,
            },
        );
        drop(instances);
        if let Err(error) = self.register_proxy_worker(&vm_id, generation, proxy.clone()) {
            self.evict_instance(&vm_id, generation);
            let _reaper = instance_reaper::kill_and_reap(child, retirement);
            return Err(error.context("register proxy worker"));
        }
        if let Err(error) = tokio::runtime::Handle::current().block_on(
            crate::service_runtime::telemetry_export::grant_proxy_metric_broker(self, &vm_id, generation, &proxy),
        ) {
            self.evict_instance(&vm_id, generation);
            let _reaper = instance_reaper::kill_and_reap(child, retirement);
            return Err(error);
        }
        let retirement = retirement.with_brokers([
            upstream_broker.start(upstream_grant),
            proxy_upstream_broker.start(proxy_upstream_grant),
        ]);
        let _reaper = instance_reaper::spawn_exit_reaper(
            child,
            vm_id.clone(),
            name,
            Arc::clone(self),
            uds_path,
            session_dir,
            retirement,
        );
        guest_lifecycle::spawn_suspend_listener(Arc::clone(self), vm_id.clone(), generation);
        // A resumed member's networks get their links back.
        switches::plug_memberships(Arc::clone(self), vm_id.clone());
        Ok(vm_id)
    }
}

#[cfg(test)]
mod tests;
