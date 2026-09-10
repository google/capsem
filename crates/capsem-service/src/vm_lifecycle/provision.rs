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
}
