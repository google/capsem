use super::*;

impl ServiceState {
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
                .arg("--uds-path")
                .arg(&uds_path)
                // Explicitly, because `uds_path` may have been shortened out
                // of the run tree and cannot be walked back up.
                .arg("--run-dir")
                .arg(&self.run_dir)
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
            },
        );
        drop(instances);
        let _reaper =
            instance_reaper::spawn_exit_reaper(child, vm_id.clone(), name, Arc::clone(self), uds_path, session_dir);
        Ok(vm_id)
    }
}
