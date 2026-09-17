//! Spawning capsem-process for a new VM and for a persistent VM that resumes.
//!
//! Both paths build the same command line: identity for the guest, pinned
//! assets, session directory and IPC socket. Split out
//! of main.rs by size, not by ownership -- these are `ServiceState` methods.
use super::*;

impl ServiceState {
    pub(crate) fn provision_sandbox(self: &Arc<Self>, options: ProvisionOptions) -> Result<()> {
        let ProvisionOptions {
            id,
            name,
            profile_id,
            ram_mb,
            cpus,
            scratch_disk_size_gb,
            version_override,
            persistent,
            env,
            from,
            description,
            auto_snapshot_max,
            manual_snapshot_max,
            auto_snapshot_interval,
        } = options;
        validate_profile_route_id(profile_id.clone()).map_err(|error| anyhow!("invalid profile_id: {}", error.1))?;

        let vm_settings = capsem_core::net::policy_config::load_merged_vm_settings();
        let max_concurrent_vms = vm_settings.max_concurrent_vms.unwrap_or(10) as usize;

        if !(1..=8).contains(&cpus) {
            return Err(anyhow!("cpus must be between 1 and 8"));
        }
        if !(256..=16384).contains(&ram_mb) {
            return Err(anyhow!("ram_mb must be between 256 and 16384"));
        }

        // Persistent VMs: validate the human display name and reject duplicate names.
        if persistent {
            validate_vm_name(name)?;
            let registry = self.persistent_registry.lock().unwrap();
            if registry.contains(name) {
                return Err(anyhow!(
                    "persistent VM \"{}\" already exists. Use `capsem resume {}` to reconnect.",
                    name,
                    name
                ));
            }
        }

        // Stale-record reclamation only runs when we'd otherwise reject the
        // provision. The probe acquires the instances mutex that many other
        // handlers contend for, and with the lock-released-before-fs-io
        // contract of `cleanup_stale_instances` the cost is minimal, but
        // this still skips an avoidable acquisition on the common path.
        let cleanup_needed = {
            let instances = self.instances.lock().unwrap();
            instances.contains_key(id) || instances.len() >= max_concurrent_vms
        };
        if cleanup_needed {
            self.cleanup_stale_instances();
        }

        {
            let instances = self.instances.lock().unwrap();
            if instances.contains_key(id) {
                return Err(anyhow!("sandbox already exists: {}", id));
            }
            if instances.len() >= max_concurrent_vms {
                return Err(anyhow!(
                    "maximum number of concurrent VMs reached ({})",
                    max_concurrent_vms
                ));
            }
        }

        // Validate source sandbox if --from provided
        let source_entry = if let Some(ref from_name) = from {
            let registry = self.persistent_registry.lock().unwrap();
            let entry = registry
                .get(from_name)
                .ok_or_else(|| anyhow!("source sandbox '{}' not found", from_name))?
                .clone();
            drop(registry);
            if entry.profile_id != profile_id {
                return Err(anyhow!(
                    "source sandbox '{}' uses profile '{}', not '{}'",
                    from_name,
                    entry.profile_id,
                    profile_id
                ));
            }
            Some(entry)
        } else {
            None
        };

        // If cloning from a source sandbox, inherit its base_version.
        let version = if let Some(ref entry) = source_entry {
            entry.base_version.clone()
        } else {
            version_override.unwrap_or_else(|| self.current_version.clone())
        };

        info!(id, version, persistent, from, "provision_sandbox called");

        let uds_path = self.instance_socket_path(id)?;

        // Persistent VMs go in persistent/, ephemeral in sessions/
        let session_dir = if persistent {
            self.run_dir.join("persistent").join(id)
        } else {
            self.run_dir.join("sessions").join(id)
        };

        info!(uds_path = %uds_path.display(), "using uds_path");
        info!(session_dir = %session_dir.display(), "using session_dir");

        let _ = std::fs::create_dir_all(uds_path.parent().unwrap());
        let _ = std::fs::create_dir_all(&session_dir);

        // If cloning from a source sandbox, clone its state into the new session directory
        if let Some(ref entry) = source_entry {
            info!(from = entry.name, session_dir = %session_dir.display(), "cloning session from source sandbox");
            capsem_core::auto_snapshot::clone_sandbox_state(&entry.session_dir, &session_dir)
                .context("failed to clone sandbox state")?;
        }

        let runtime_profile = self.cached_profile_for_runtime(&profile_id)?;
        let active_profile_path = self.materialize_active_profile(&runtime_profile, &session_dir)?;
        let profile = runtime_profile.config();
        let profile_revision = profile.revision.clone();
        let profile_payload_hash = profile_payload_hash(profile)?;
        let asset_pins = profile_asset_pins(profile)?;
        self.validate_profile_pins(profile, &profile_revision, &profile_payload_hash, &asset_pins)?;
        let resolved = self.resolve_profile_asset_paths(profile)?;

        info!(process_binary = %self.process_binary.display(), exists = self.process_binary.exists(), "checking process_binary");

        info!(id, version, asset_version = %resolved.asset_version, "spawning capsem-process");

        let mut child_cmd = tokio::process::Command::new(&self.process_binary);
        if !self.process_binary.exists() {
            info!("process_binary does not exist at absolute path, trying cache/target/cargo/debug/capsem-process");
            child_cmd = tokio::process::Command::new("cache/target/cargo/debug/capsem-process");
        }

        let process_log_path = session_dir.join("process.log");
        let owner_secret = private_routes::mint_owner_secret(&session_dir)?;
        let process_log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&process_log_path)
            .context("failed to open process.log")?;

        // Inject VM identity so the guest knows its own name/ID. Anonymous
        // sessions have a generated display name for UI lists, but the guest
        // hostname must remain the opaque route id.
        let guest_name = if persistent { name } else { id };
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_ID={}", id));
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_NAME={}", guest_name));
        child_cmd.arg("--vm-name").arg(guest_name);

        // Add --env KEY=VALUE args for each user-specified env var
        if let Some(ref env_vars) = env {
            for (k, v) in env_vars {
                child_cmd.arg("--env").arg(format!("{}={}", k, v));
            }
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
        // W4: propagate trace context to the child process.
        // CAPSEM_VM_ID, CAPSEM_TRACE_ID, TRACEPARENT, TRACESTATE.
        for (k, v) in capsem_foundation::telemetry::child_trace_env(id) {
            child_cmd.env(k, v);
        }

        let process_spawn_span = tracing::debug_span!(
            target: "capsem.launch",
            capsem_foundation::telemetry::LAUNCH_PROCESS_SPAWN_SPAN,
            boot_mode = "provision",
            status = tracing::field::Empty,
        );
        let mut child = match process_spawn_span.in_scope(|| {
            child_cmd
                .env(
                    "RUST_LOG",
                    std::env::var("RUST_LOG")
                        .unwrap_or_else(|_| capsem_foundation::telemetry::with_subsys_targets("capsem=info")),
                )
                .arg("--id")
                .arg(id)
                .arg("--assets-dir")
                .arg(&self.assets_dir)
                .arg("--rootfs")
                .arg(&resolved.rootfs)
                .arg("--kernel")
                .arg(&resolved.kernel)
                .arg("--initrd")
                .arg(&resolved.initrd)
                // The profile's own pins. Boot verifies against these, never
                // against a channel-wide pointer that can only name one profile.
                .arg("--expected-kernel-hash")
                .arg(&asset_pins.kernel.hash)
                .arg("--expected-initrd-hash")
                .arg(&asset_pins.initrd.hash)
                .arg("--expected-rootfs-hash")
                .arg(&asset_pins.rootfs.hash)
                .arg("--session-dir")
                .arg(&session_dir)
                .arg("--active-profile")
                .arg(&active_profile_path)
                .arg("--cpus")
                .arg(cpus.to_string())
                .arg("--ram-mb")
                .arg(ram_mb.to_string())
                .arg("--scratch-disk-size-gb")
                .arg(scratch_disk_size_gb.to_string())
                .arg("--auto-snapshot-max")
                .arg(auto_snapshot_max.to_string())
                .arg("--manual-snapshot-max")
                .arg(manual_snapshot_max.to_string())
                .arg("--auto-snapshot-interval")
                .arg(auto_snapshot_interval.to_string())
                .arg("--uds-path")
                .arg(&uds_path)
                // Explicitly, because `uds_path` may have been shortened out
                // of the run tree and cannot be walked back up.
                .arg("--run-dir")
                .arg(&self.run_dir)
                .arg("--service-socket")
                .arg(&self.service_socket)
                .stdout(std::process::Stdio::from(process_log_file.try_clone()?))
                .stderr(std::process::Stdio::from(process_log_file))
                .spawn()
        }) {
            Ok(child) => {
                process_spawn_span.record("status", "ok");
                child
            }
            Err(error) => {
                process_spawn_span.record("status", "error");
                return Err(anyhow::Error::new(error).context("failed to spawn capsem-process"));
            }
        };

        let pid = child.id().unwrap_or(0);
        info!(id, pid, version, asset_version = %resolved.asset_version, "capsem-process spawned");

        if let Err(error) = self.record_session_index_start(
            id,
            persistent,
            scratch_disk_size_gb,
            ram_mb,
            Some(&asset_pins.rootfs.hash),
            Some(&version),
            from.as_deref(),
        ) {
            let _ = child.start_kill();
            return Err(error.context("failed to record main.db session start"));
        }

        if persistent {
            let registration = self.persistent_registry.lock().unwrap().register(PersistentVmEntry {
                auto_snapshot_max: Some(auto_snapshot_max),
                id: id.to_string(),
                name: name.to_string(),
                profile_id: profile_id.clone(),
                profile_revision: profile_revision.clone(),
                profile_payload_hash: profile_payload_hash.clone(),
                asset_pins: asset_pins.clone(),
                ram_mb,
                cpus,
                base_version: version.clone(),
                created_at: format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                ),
                session_dir: session_dir.clone(),
                forked_from: from.clone(),
                description,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: env.clone(),
            });
            if let Err(error) = registration {
                instance_reaper::kill_and_reap(child);
                return Err(error);
            }
        }

        if session_db_path_for_session_dir(&session_dir).exists() {
            if let Err(error) = self.register_session_db_handle(id, &session_dir) {
                instance_reaper::kill_and_reap(child);
                return Err(error);
            }
        } else {
            info!(
                vm_id = id,
                operation = "defer_session_db_handle_registration",
                session_dir = %session_dir.display(),
                "session DB not present yet; route will register the external reader lazily"
            );
        }

        let mut instances = self.instances.lock().unwrap();
        instances.insert(
            id.to_string(),
            InstanceInfo {
                id: id.to_string(),
                name: name.to_string(),
                profile_id,
                profile_revision,
                profile_payload_hash,
                asset_pins,
                pid,
                uds_path: uds_path.clone(),
                session_dir: session_dir.clone(),
                ram_mb,
                cpus,
                start_time: std::time::Instant::now(),
                base_version: version,
                persistent,
                env,
                forked_from: from,
                owner_secret,
            },
        );
        drop(instances);
        let _reaper = instance_reaper::spawn_exit_reaper(
            child,
            id.to_string(),
            name.to_string(),
            Arc::clone(self),
            uds_path,
            session_dir,
        );

        Ok(())
    }

    /// Resume a stopped persistent VM by re-spawning capsem-process against its
    /// existing session directory.
    pub(crate) fn resume_sandbox(
        self: &Arc<Self>,
        id: &str,
        ram_mb_override: Option<u64>,
        cpus_override: Option<u32>,
    ) -> Result<String> {
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

        self.validate_persistent_profile_authority(&entry)?;
        let active_profile_path = self.persistent_active_profile_path(&entry)?;
        let scratch_disk_size_gb = self.persistent_scratch_disk_size_gb(&entry)?;
        let resolved = self.resolve_pinned_asset_paths(&entry.asset_pins)?;
        self.validate_pinned_asset_files(&resolved, &entry.asset_pins)?;

        let process_log_path = entry.session_dir.join("process.log");
        let owner_secret = private_routes::mint_owner_secret(&entry.session_dir)?;
        let process_log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&process_log_path)
            .context("failed to open process.log")?;

        let mut child_cmd = tokio::process::Command::new(&self.process_binary);
        if !self.process_binary.exists() {
            child_cmd = tokio::process::Command::new("cache/target/cargo/debug/capsem-process");
        }

        // Inject VM identity so the guest knows its own name/ID.
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_ID={}", vm_id));
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_NAME={}", name));
        child_cmd.arg("--vm-name").arg(&name);

        // Replay user-provided env vars so they survive stop/resume cycles.
        if let Some(ref env_vars) = entry.env {
            for (k, v) in env_vars {
                child_cmd.arg("--env").arg(format!("{}={}", k, v));
            }
        }

        // Pass checkpoint path for warm restore from suspended state only
        // when the completion marker proves save_state + fsync finished.
        if entry.suspended {
            if let Some(ref cp) = entry.checkpoint_path {
                let full_checkpoint = entry.session_dir.join(cp);
                let complete = checkpoint_complete_path(&full_checkpoint);
                if full_checkpoint.exists() && complete.exists() {
                    child_cmd.arg("--checkpoint-path").arg(&full_checkpoint);
                    info!(name, checkpoint = %full_checkpoint.display(), "warm restore from checkpoint");
                } else {
                    tracing::warn!(name, checkpoint = %full_checkpoint.display(), complete = %complete.display(), "checkpoint incomplete, cold booting");
                }
            }
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

        let process_spawn_span = tracing::debug_span!(
            target: "capsem.launch",
            capsem_foundation::telemetry::LAUNCH_PROCESS_SPAWN_SPAN,
            boot_mode = "resume",
            status = tracing::field::Empty,
        );
        let child = match process_spawn_span.in_scope(|| {
            child_cmd
                .env(
                    "RUST_LOG",
                    std::env::var("RUST_LOG")
                        .unwrap_or_else(|_| capsem_foundation::telemetry::with_subsys_targets("capsem=info")),
                )
                .arg("--id")
                .arg(&vm_id)
                .arg("--assets-dir")
                .arg(&self.assets_dir)
                .arg("--rootfs")
                .arg(&resolved.rootfs)
                .arg("--kernel")
                .arg(&resolved.kernel)
                .arg("--initrd")
                .arg(&resolved.initrd)
                // The profile's own pins. Boot verifies against these, never
                // against a channel-wide pointer that can only name one profile.
                .arg("--expected-kernel-hash")
                .arg(&entry.asset_pins.kernel.hash)
                .arg("--expected-initrd-hash")
                .arg(&entry.asset_pins.initrd.hash)
                .arg("--expected-rootfs-hash")
                .arg(&entry.asset_pins.rootfs.hash)
                .arg("--session-dir")
                .arg(&entry.session_dir)
                .arg("--active-profile")
                .arg(&active_profile_path)
                .arg("--cpus")
                .arg(cpus.to_string())
                .arg("--ram-mb")
                .arg(ram_mb.to_string())
                .arg("--scratch-disk-size-gb")
                .arg(scratch_disk_size_gb.to_string())
                .arg("--auto-snapshot-max")
                .arg(
                    entry
                        .auto_snapshot_max
                        .or_else(|| {
                            self.cached_profile_config(&entry.profile_id)
                                .map(|p| p.vm.snapshots.auto_max)
                                .ok()
                        })
                        .unwrap_or(10)
                        .to_string(),
                )
                .arg("--manual-snapshot-max")
                .arg(
                    self.cached_profile_config(&entry.profile_id)
                        .map(|p| p.vm.snapshots.manual_max)
                        .unwrap_or(12)
                        .to_string(),
                )
                .arg("--auto-snapshot-interval")
                .arg(
                    self.cached_profile_config(&entry.profile_id)
                        .map(|p| p.vm.snapshots.auto_interval)
                        .unwrap_or(300)
                        .to_string(),
                )
                .arg("--uds-path")
                .arg(&uds_path)
                // Explicitly, because `uds_path` may have been shortened out
                // of the run tree and cannot be walked back up.
                .arg("--run-dir")
                .arg(&self.run_dir)
                .arg("--service-socket")
                .arg(&self.service_socket)
                .stdout(std::process::Stdio::from(process_log_file.try_clone()?))
                .stderr(std::process::Stdio::from(process_log_file))
                .spawn()
        }) {
            Ok(child) => {
                process_spawn_span.record("status", "ok");
                child
            }
            Err(error) => {
                process_spawn_span.record("status", "error");
                return Err(anyhow::Error::new(error).context("failed to spawn capsem-process"));
            }
        };

        let pid = child.id().unwrap_or(0);
        info!(name, pid, "capsem-process resumed");

        if session_db_path_for_session_dir(&entry.session_dir).exists() {
            if let Err(error) = self.register_session_db_handle(&vm_id, &entry.session_dir) {
                instance_reaper::kill_and_reap(child);
                return Err(error);
            }
        } else {
            info!(
                vm_id = vm_id,
                operation = "defer_session_db_handle_registration",
                session_dir = %entry.session_dir.display(),
                "session DB not present yet; route will register the external reader lazily"
            );
        }

        let session_dir = entry.session_dir.clone();
        let mut instances = self.instances.lock().unwrap();
        instances.insert(
            vm_id.clone(),
            InstanceInfo {
                id: vm_id.clone(),
                name: entry.name.clone(),
                profile_id: entry.profile_id.clone(),
                profile_revision: entry.profile_revision.clone(),
                profile_payload_hash: entry.profile_payload_hash.clone(),
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
                forked_from: entry.forked_from,
                owner_secret,
            },
        );
        drop(instances);
        let _reaper =
            instance_reaper::spawn_exit_reaper(child, vm_id.clone(), name, Arc::clone(self), uds_path, session_dir);
        // A resumed member's networks get their links back.
        switches::plug_memberships(Arc::clone(self), vm_id.clone());
        Ok(vm_id)
    }
}
