use super::*;
use capsem_core::net::policy_config::{ResolvedSetting, SettingValue};
use clap::Parser;

fn git_short_head() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[test]
fn the_embedded_build_hash_carries_the_real_source_revision() {
    // build.rs treats a failing `git rev-parse` as absent rather than
    // fatal and embeds "unknown", so any environment that cannot read the
    // repository -- a Linux bind mount whose owner is not the container
    // user, most recently -- yields a binary with no source identity.
    // check-build-provenance.sh catches that when packaging; without this
    // the invariant lives entirely outside the test suite.
    let Some(head) = git_short_head() else {
        // Building from an exported tarball has no revision to carry, and
        // "unknown" is the honest answer there.
        return;
    };
    let embedded = env!("CAPSEM_BUILD_HASH");

    assert!(
        !embedded.starts_with("unknown"),
        "build hash lost its source revision: {embedded}"
    );
    assert!(
        embedded.starts_with(&head),
        "build hash {embedded} does not carry HEAD {head}"
    );
}

#[test]
fn cli_runtime_paths_are_derived_from_one_run_dir() {
    let run_dir = tempfile::tempdir().unwrap();
    let paths = cli_runtime_paths_from_run_dir(run_dir.path());

    assert_eq!(paths.service_socket, run_dir.path().join("service.sock"));
    assert_eq!(paths.gateway_port, run_dir.path().join("gateway.port"));
    assert_eq!(paths.gateway_token, run_dir.path().join("gateway.token"));
}

fn update_track(
    current: Option<&str>,
    latest: Option<&str>,
    state: UpdateTrackState,
    available: bool,
) -> client::UpdateTrackStatus {
    client::UpdateTrackStatus {
        current: current.map(ToOwned::to_owned),
        latest: latest.map(ToOwned::to_owned),
        blocked_reason: None,
        update_available: available,
        state,
        compatibility: client::UpdateCompatibilityState::Compatible,
    }
}

fn session(status: client::VmLifecycleState) -> SessionInfo {
    SessionInfo {
        id: "vm1".into(),
        profile_id: "code".into(),
        name: Some("dev".into()),
        pid: 0,
        status,
        persistent: true,
        ram_mb: None,
        cpus: None,
        version: None,
        forked_from: None,
        description: None,
        created_at: None,
        uptime_secs: None,
        total_input_tokens: None,
        total_output_tokens: None,
        total_estimated_cost: None,
        total_tool_calls: None,
        total_requests: None,
        allowed_requests: None,
        denied_requests: None,
        total_file_events: None,
        model_call_count: None,
        last_error: None,
        can_resume: false,
        resume_blocked_reason: None,
    }
}

#[test]
fn session_blocked_reason_distils_a_crashed_boot_to_its_error_line() {
    let mut vm = session(client::VmLifecycleState::Defunct);
    vm.last_error = Some(
        "INFO capsem_process: booting\n\
         ERROR capsem_process: failed to build VmConfig: rootfs hash mismatch\n"
            .into(),
    );

    assert_eq!(
        session_blocked_reason(&vm),
        Some("ERROR capsem_process: failed to build VmConfig: rootfs hash mismatch")
    );
}

#[test]
fn session_blocked_reason_explains_a_stopped_vm_the_service_will_not_resume() {
    // The reachable case `capsem list` used to render as a bare "Stopped" row:
    // the VM never crashed, so there is no last_error, but asset validation
    // fails and the service refuses to start it.
    let mut vm = session(client::VmLifecycleState::Stopped);
    vm.resume_blocked_reason = Some("rootfs asset file is missing".into());

    assert_eq!(
        session_blocked_reason(&vm),
        Some("rootfs asset file is missing")
    );
}

#[test]
fn session_blocked_reason_stays_quiet_for_a_healthy_session() {
    assert_eq!(session_blocked_reason(&session(client::VmLifecycleState::Running)), None);

    let mut resumable = session(client::VmLifecycleState::Stopped);
    resumable.can_resume = true;
    assert_eq!(session_blocked_reason(&resumable), None);
}

fn base_update_status() -> UpdateStatusResponse {
    UpdateStatusResponse {
        checked_at: Some(1_718_444_400),
        channel_url: Some("https://release.capsem.org/health.json".into()),
        channel_hash: None,
        validation_status: Some("valid".into()),
        validation_error: None,
        stale: false,
        last_error: None,
        binary: update_track(
            Some("1.4.0"),
            Some("1.4.0"),
            UpdateTrackState::Current,
            false,
        ),
        assets: update_track(
            Some("2026.0627.1"),
            Some("2026.0627.1"),
            UpdateTrackState::Current,
            false,
        ),
        profiles: update_track(
            Some("profiles-1"),
            Some("profiles-1"),
            UpdateTrackState::Current,
            false,
        ),
        images: update_track(None, None, UpdateTrackState::NotPublished, false),
        supply_chain: client::SupplyChainEvidence::default(),
    }
}

#[test]
fn update_status_lines_separate_available_and_blocked_tracks() {
    let mut status = base_update_status();
    status.binary = update_track(
        Some("1.4.0"),
        Some("1.4.1"),
        UpdateTrackState::UpdateAvailable,
        true,
    );
    status.profiles = update_track(
        Some("profiles-1"),
        Some("profiles-2"),
        UpdateTrackState::UpdateAvailable,
        true,
    );
    status.assets.blocked_reason = Some("requires binary 1.4.1 or newer".into());
    status.images.blocked_reason = Some("image catalog not published".into());

    let lines = update_status_lines(&status);

    assert_eq!(
        lines[0],
        "Updates:   available (binary 1.4.0 -> 1.4.1; profiles profiles-1 -> profiles-2); blocked (assets, images)"
    );
    assert!(lines.contains(&"Channel:   https://release.capsem.org/health.json".into()));
    assert!(lines.contains(&"Assets:    blocked (requires binary 1.4.1 or newer)".into()));
    assert!(lines.contains(&"Images:    blocked (image catalog not published)".into()));
}

#[test]
fn update_status_lines_does_not_invent_track_or_cache_state_when_current() {
    let mut status = base_update_status();
    status.stale = true;
    status.assets.state = UpdateTrackState::Unknown;

    let lines = update_status_lines(&status);

    assert_eq!(lines[0], "Updates:   current");
    assert!(!lines.join("\n").contains("not published"));
    assert!(!lines.join("\n").contains("cache"));
}

// -----------------------------------------------------------------------
// CLI parsing
// -----------------------------------------------------------------------

#[test]
fn parse_no_subcommand() {
    let cli = Cli::try_parse_from(["capsem"]);
    assert!(cli.is_ok());
    let cli = cli.unwrap();
    assert!(cli.command.is_none());
}

#[test]
fn parse_create_with_name() {
    let cli = Cli::parse_from(["capsem", "create", "-n", "my-vm"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { name, ram, cpu, .. }) => {
            assert_eq!(name, Some("my-vm".into()));
            assert_eq!(ram, 4);
            assert_eq!(cpu, 4);
        }
        _ => panic!("expected Create"),
    }
}

#[test]
fn cli_create_accepts_profile() {
    let cli = Cli::parse_from(["capsem", "create", "--profile", "co-work"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { profile, .. }) => {
            assert_eq!(profile, "co-work");
        }
        _ => panic!("expected Create"),
    }
}

#[test]
fn cli_run_accepts_profile() {
    let cli = Cli::parse_from(["capsem", "run", "echo ok", "--profile", "co-work"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Run { profile, .. }) => {
            assert_eq!(profile, "co-work");
        }
        _ => panic!("expected Run"),
    }
}

#[test]
fn cli_mcp_commands_accept_profile() {
    let cases = [
        vec!["capsem", "mcp", "servers", "--profile", "co-work"],
        vec!["capsem", "mcp", "tools", "--profile", "co-work"],
        vec!["capsem", "mcp", "refresh", "--profile", "co-work"],
        vec![
            "capsem",
            "mcp",
            "call",
            "server__tool",
            "--profile",
            "co-work",
        ],
    ];

    for args in cases {
        let cli = Cli::parse_from(args);
        let profile = match cli.command.unwrap() {
            Commands::Mcp(McpCommands::Servers { profile })
            | Commands::Mcp(McpCommands::Tools { profile, .. })
            | Commands::Mcp(McpCommands::Refresh { profile })
            | Commands::Mcp(McpCommands::Call { profile, .. }) => profile,
            _ => panic!("expected MCP command"),
        };
        assert_eq!(profile, "co-work");
    }
}

#[test]
fn parse_create_ephemeral() {
    let cli = Cli::parse_from(["capsem", "create"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { name, .. }) => {
            assert_eq!(name, None);
        }
        _ => panic!("expected Create"),
    }
}

#[test]
fn parse_create_with_resources() {
    let cli = Cli::parse_from(["capsem", "create", "--ram", "8", "--cpu", "2"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { ram, cpu, .. }) => {
            assert_eq!(ram, 8);
            assert_eq!(cpu, 2);
        }
        _ => panic!("expected Create"),
    }
}

#[test]
fn parse_resume() {
    let cli = Cli::parse_from(["capsem", "resume", "mydev"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Resume { name }) => assert_eq!(name, "mydev"),
        _ => panic!("expected Resume"),
    }
}

#[test]
fn parse_attach_alias_for_resume() {
    let cli = Cli::parse_from(["capsem", "attach", "mydev"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Resume { name }) => assert_eq!(name, "mydev"),
        _ => panic!("expected Resume via attach alias"),
    }
}

#[test]
fn parse_suspend() {
    let cli = Cli::parse_from(["capsem", "suspend", "vm-123"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Suspend { session }) => {
            assert_eq!(session, "vm-123")
        }
        _ => panic!("expected Suspend"),
    }
}

#[test]
fn parse_shell_positional() {
    let cli = Cli::parse_from(["capsem", "shell", "my-vm"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Shell { session, name }) => {
            assert_eq!(session, Some("my-vm".into()));
            assert_eq!(name, None);
        }
        _ => panic!("expected Shell"),
    }
}

#[test]
fn parse_shell_by_name() {
    let cli = Cli::parse_from(["capsem", "shell", "-n", "mydev"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Shell { name, session }) => {
            assert_eq!(name, Some("mydev".into()));
            assert_eq!(session, None);
        }
        _ => panic!("expected Shell"),
    }
}

#[test]
fn parse_shell_bare() {
    // Bare `capsem shell` = temp session + auto-destroy
    let cli = Cli::parse_from(["capsem", "shell"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Shell { name, session }) => {
            assert_eq!(name, None);
            assert_eq!(session, None);
        }
        _ => panic!("expected Shell"),
    }
}

#[test]
fn parse_persist() {
    let cli = Cli::parse_from(["capsem", "persist", "vm-123", "mydev"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Persist { session, name }) => {
            assert_eq!(session, "vm-123");
            assert_eq!(name, "mydev");
        }
        _ => panic!("expected Persist"),
    }
}

#[test]
fn parse_purge() {
    let cli = Cli::parse_from(["capsem", "purge"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Purge { all }) => assert!(!all),
        _ => panic!("expected Purge"),
    }
}

#[test]
fn parse_purge_all() {
    let cli = Cli::parse_from(["capsem", "purge", "--all"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Purge { all }) => assert!(all),
        _ => panic!("expected Purge --all"),
    }
}

#[test]
fn purge_summary_mentions_broken_persistent_for_default_purge() {
    let result = PurgeResponse {
        purged: 2,
        persistent_purged: 1,
        ephemeral_purged: 1,
    };
    assert_eq!(
        purge_summary_message(&result, false),
        "[*] Purged 2 sessions (1 broken persistent, 1 temporary)."
    );
}

#[test]
fn purge_summary_keeps_temporary_only_message_when_no_defunct_persistent() {
    let result = PurgeResponse {
        purged: 3,
        persistent_purged: 0,
        ephemeral_purged: 3,
    };
    assert_eq!(
        purge_summary_message(&result, false),
        "[*] Purged 3 temporary sessions."
    );
}

#[test]
fn parse_run() {
    let cli = Cli::parse_from(["capsem", "run", "echo hello"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Run {
            command,
            profile,
            timeout,
            env,
        }) => {
            assert_eq!(command, "echo hello");
            assert_eq!(profile, "code");
            assert_eq!(timeout, None);
            assert!(env.is_empty());
        }
        _ => panic!("expected Run"),
    }
}

#[test]
fn parse_run_with_timeout() {
    let cli = Cli::parse_from(["capsem", "run", "--timeout", "120", "ls -la"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Run {
            command,
            profile,
            timeout,
            env,
        }) => {
            assert_eq!(command, "ls -la");
            assert_eq!(profile, "code");
            assert_eq!(timeout, Some(120));
            assert!(env.is_empty());
        }
        _ => panic!("expected Run"),
    }
}

#[test]
fn parse_list() {
    let cli = Cli::parse_from(["capsem", "list"]);
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Session(SessionCommands::List { quiet: false })
    ));
}

#[test]
fn parse_list_quiet() {
    let cli = Cli::parse_from(["capsem", "list", "-q"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::List { quiet }) => assert!(quiet),
        _ => panic!("expected List"),
    }
}

#[test]
fn parse_list_quiet_long() {
    let cli = Cli::parse_from(["capsem", "list", "--quiet"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::List { quiet }) => assert!(quiet),
        _ => panic!("expected List"),
    }
}

#[test]
fn parse_status() {
    // `capsem status` is now the service status command
    let cli = Cli::parse_from(["capsem", "status"]);
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Misc(MiscCommands::Status)
    ));
}

#[test]
fn service_control_commands_do_not_start_background_update_work() {
    for args in [
        &["capsem", "install"][..],
        &["capsem", "status"][..],
        &["capsem", "start"][..],
        &["capsem", "stop"][..],
        &["capsem", "version"][..],
        &["capsem", "update", "--assets", "--channel", "stable"][..],
        &["capsem", "debug"][..],
        &["capsem", "completions", "zsh"][..],
        &["capsem", "uninstall", "--yes"][..],
    ] {
        let cli = Cli::parse_from(args);
        let command = cli.command.as_ref().expect("parsed command");
        assert!(
            !should_refresh_update_cache_for_command(command),
            "{args:?} must stay a pure local control command"
        );
    }
}

#[test]
fn session_commands_may_refresh_update_cache() {
    let cli = Cli::parse_from(["capsem", "list"]);
    let command = cli.command.as_ref().expect("parsed command");
    assert!(should_refresh_update_cache_for_command(command));
}

fn resolved_auto_update(value: SettingValue) -> ResolvedSetting {
    ResolvedSetting {
        id: "app.auto_update".to_string(),
        category: "App".to_string(),
        name: "Auto-check for updates".to_string(),
        description: String::new(),
        setting_type: capsem_core::net::policy_config::SettingType::Bool,
        default_value: SettingValue::Bool(true),
        effective_value: value,
        source: capsem_core::net::policy_config::PolicySource::User,
        modified: Some("test".to_string()),
        corp_locked: false,
        enabled_by: None,
        enabled: true,
        metadata: capsem_core::net::policy_config::SettingMetadata::default(),
        collapsed: false,
        history: Vec::new(),
    }
}

#[test]
fn auto_update_defaults_to_enabled_when_setting_is_absent_or_malformed() {
    assert!(auto_update_enabled_from_resolved(&[]));
    assert!(auto_update_enabled_from_resolved(&[resolved_auto_update(
        SettingValue::Text("false".to_string())
    )]));
}

#[test]
fn auto_update_setting_can_disable_background_refresh() {
    let off = [resolved_auto_update(SettingValue::Bool(false))];
    assert!(!auto_update_enabled_from_resolved(&off));

    let on = [resolved_auto_update(SettingValue::Bool(true))];
    assert!(auto_update_enabled_from_resolved(&on));
}

#[test]
fn app_auto_update_false_disables_background_refresh_from_settings_file() {
    let _guard = lock_test_env();
    let capsem_home = tempfile::tempdir().unwrap();
    let _capsem_paths = capsem_core::paths::CapsemPathsGuard::redirect(capsem_home.path());
    std::fs::write(
        capsem_home.path().join("settings.toml"),
        "[settings.\"app.auto_update\"]\nvalue = false\nmodified = \"test\"\n",
    )
    .unwrap();

    let cli = Cli::parse_from(["capsem", "list"]);
    let command = cli.command.as_ref().expect("parsed command");
    assert!(!should_start_background_update_refresh(Some(command)));
    assert!(!should_start_background_update_refresh(None));

}

#[test]
fn service_control_commands_do_not_cross_service_api_boundary() {
    for args in [
        &["capsem", "install"][..],
        &["capsem", "status"][..],
        &["capsem", "start"][..],
        &["capsem", "stop"][..],
        &["capsem", "debug"][..],
        &["capsem", "completions", "zsh"][..],
        &["capsem", "uninstall", "--yes"][..],
        &["capsem", "update", "--yes"][..],
    ] {
        let cli = Cli::parse_from(args);
        let command = cli.command.as_ref().expect("parsed command");
        assert!(
            command_is_handled_before_service_api(command),
            "{args:?} must be handled before UDS/service API construction so service control cannot depend on profile, status, or credential-store readiness"
        );
    }
}

#[test]
fn session_commands_cross_service_api_boundary() {
    for args in [
        &["capsem", "list"][..],
        &["capsem", "exec", "code-1", "true"][..],
        &["capsem", "assets", "status"][..],
    ] {
        let cli = Cli::parse_from(args);
        let command = cli.command.as_ref().expect("parsed command");
        assert!(
            !command_is_handled_before_service_api(command),
            "{args:?} should keep using the service API"
        );
    }
}

#[test]
fn parse_debug_aliases_support_bundle() {
    let cli = Cli::parse_from(["capsem", "debug"]);
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Misc(MiscCommands::SupportBundle { .. })
    ));
}

#[test]
fn parse_uds_path_override() {
    let cli = Cli::parse_from(["capsem", "--uds-path", "/tmp/test.sock", "list"]);
    assert_eq!(cli.uds_path, Some(PathBuf::from("/tmp/test.sock")));
}

#[test]
fn parse_uds_path_default_none() {
    let cli = Cli::parse_from(["capsem", "list"]);
    assert_eq!(cli.uds_path, None);
}

// -----------------------------------------------------------------------
// RAM conversion
// -----------------------------------------------------------------------

#[test]
fn ram_gb_to_mb_conversion() {
    let ram_gb: u64 = 4;
    assert_eq!(ram_gb * 1024, 4096);
}

// -----------------------------------------------------------------------
// New commands: exec, delete, info, doctor
// -----------------------------------------------------------------------

#[test]
fn parse_exec() {
    let cli = Cli::parse_from(["capsem", "exec", "my-vm", "echo hello"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Exec {
            session,
            command,
            timeout,
        }) => {
            assert_eq!(session, "my-vm");
            assert_eq!(command, "echo hello");
            assert_eq!(timeout, None);
        }
        _ => panic!("expected Exec"),
    }
}

#[test]
fn parse_exec_with_timeout() {
    let cli = Cli::parse_from(["capsem", "exec", "--timeout", "120", "my-vm", "make build"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Exec {
            session,
            command,
            timeout,
        }) => {
            assert_eq!(session, "my-vm");
            assert_eq!(command, "make build");
            assert_eq!(timeout, Some(120));
        }
        _ => panic!("expected Exec"),
    }
}

#[test]
fn parse_delete() {
    let cli = Cli::parse_from(["capsem", "delete", "vm-123"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Delete { session }) => assert_eq!(session, "vm-123"),
        _ => panic!("expected Delete"),
    }
}

#[test]
fn parse_info() {
    let cli = Cli::parse_from(["capsem", "info", "vm-1"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Info { session, json }) => {
            assert_eq!(session, "vm-1");
            assert!(!json);
        }
        _ => panic!("expected Info"),
    }
}

#[test]
fn parse_info_json() {
    let cli = Cli::parse_from(["capsem", "info", "--json", "vm-1"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Info { session, json }) => {
            assert_eq!(session, "vm-1");
            assert!(json);
        }
        _ => panic!("expected Info --json"),
    }
}

#[test]
fn parse_logs_with_tail() {
    let cli = Cli::parse_from(["capsem", "logs", "--tail", "50", "vm-1"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Logs { session, tail }) => {
            assert_eq!(session, "vm-1");
            assert_eq!(tail, Some(50));
        }
        _ => panic!("expected Logs"),
    }
}

#[test]
fn parse_logs_without_tail() {
    let cli = Cli::parse_from(["capsem", "logs", "vm-1"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Logs { session, tail }) => {
            assert_eq!(session, "vm-1");
            assert_eq!(tail, None);
        }
        _ => panic!("expected Logs"),
    }
}

#[test]
fn parse_restart() {
    let cli = Cli::parse_from(["capsem", "restart", "mydev"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Restart { name }) => assert_eq!(name, "mydev"),
        _ => panic!("expected Restart"),
    }
}

#[test]
fn parse_version() {
    let cli = Cli::parse_from(["capsem", "version"]);
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Misc(MiscCommands::Version)
    ));
}

#[test]
fn parse_create_with_env() {
    let cli = Cli::parse_from(["capsem", "create", "-e", "FOO=bar", "-e", "BAZ=qux"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { env, .. }) => {
            assert_eq!(env, vec!["FOO=bar", "BAZ=qux"]);
        }
        _ => panic!("expected Create"),
    }
}

#[test]
fn parse_create_with_env_long() {
    let cli = Cli::parse_from(["capsem", "create", "--env", "API_KEY=secret123"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { env, .. }) => {
            assert_eq!(env, vec!["API_KEY=secret123"]);
        }
        _ => panic!("expected Create"),
    }
}

#[test]
fn parse_create_no_env() {
    let cli = Cli::parse_from(["capsem", "create"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { env, .. }) => {
            assert!(env.is_empty());
        }
        _ => panic!("expected Create"),
    }
}

#[test]
fn parse_doctor() {
    let cli = Cli::parse_from(["capsem", "doctor"]);
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Misc(MiscCommands::Doctor { bundle: false })
    ));
}

#[test]
fn parse_doctor_bundle_flag() {
    let cli = Cli::parse_from(["capsem", "doctor", "--bundle"]);
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Misc(MiscCommands::Doctor { bundle: true })
    ));
}

#[test]
fn parse_doctor_rejects_fast_escape_hatch() {
    let err = match Cli::try_parse_from(["capsem", "doctor", "--fast"]) {
        Ok(_) => panic!("doctor --fast must not be accepted"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("--fast"),
        "error should identify the retired flag: {err}"
    );
}

#[test]
fn doctor_mock_server_addr_is_iptables_redirect_target() {
    assert_eq!(DOCTOR_MOCK_SERVER_ADDR, "127.0.0.1:3713");
}

#[test]
fn doctor_mock_server_lock_path_matches_shared_python_launcher() {
    assert_eq!(
        DoctorMockServerLock::path_for_addr(DOCTOR_MOCK_SERVER_ADDR),
        std::env::temp_dir().join("capsem-mock-server-127-0-0-1-3713.lock")
    );
}

#[test]
fn mock_server_binary_prefers_installed_sibling() {
    let fixture = tempfile::tempdir().unwrap();
    let installed_bin = fixture.path().join("installed/bin");
    let source_bin = fixture.path().join("source/target/debug");
    std::fs::create_dir_all(&installed_bin).unwrap();
    std::fs::create_dir_all(&source_bin).unwrap();
    let executable = installed_bin.join("capsem");
    let installed_mock = installed_bin.join("capsem-mock-server");
    let source_mock = source_bin.join("capsem-mock-server");
    std::fs::write(&executable, b"capsem").unwrap();
    std::fs::write(&installed_mock, b"installed").unwrap();
    std::fs::write(&source_mock, b"source").unwrap();

    assert_eq!(
        find_mock_server_binary(
            &executable,
            &fixture.path().join("source"),
            &fixture.path().join("crates/capsem"),
        ),
        Some(installed_mock)
    );
}

#[test]
fn parse_install() {
    let cli = Cli::parse_from(["capsem", "install"]);
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Misc(MiscCommands::Install)
    ));
}

#[test]
fn parse_start() {
    let cli = Cli::parse_from(["capsem", "start"]);
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Misc(MiscCommands::Start)
    ));
}

#[test]
fn parse_stop() {
    let cli = Cli::parse_from(["capsem", "stop"]);
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Misc(MiscCommands::Stop)
    ));
}

#[test]
fn parse_setup_is_removed() {
    let err = match Cli::try_parse_from(["capsem", "setup", "--non-interactive"]) {
        Ok(_) => panic!("setup command must not parse after T5 removal"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
}

#[test]
fn parse_assets_status() {
    let cli = Cli::parse_from(["capsem", "assets", "status"]);
    match cli.command.unwrap() {
        Commands::Assets(AssetsCommands::Status { profile, json }) => {
            assert_eq!(profile, "code");
            assert!(!json);
        }
        _ => panic!("expected assets status"),
    }
}

#[test]
fn cli_default_profile_is_primary_profile() {
    assert_eq!(DEFAULT_PROFILE_ID, "code");
}

#[test]
fn status_asset_lines_are_derived_from_profiles_status_payload() {
    let payload = serde_json::json!({
        "source": "installed",
        "profile_count": 1,
        "ready_count": 1,
        "asset_manifest": {
            "origin": "package",
            "path": "/tmp/manifest.json",
            "blake3": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "assets_current": "2026.0609.1",
            "binaries_current": "1.3.0"
        },
        "profiles": [
            {
                "id": "code",
                "name": "Code",
                "ready": true,
                "current_arch": "arm64",
                "profile_payload_hash": "bbbbbbbbbbbb",
                "missing_assets": []
            }
        ]
    });

    let lines = profile_status_summary_lines(&payload);

    assert!(lines
        .iter()
        .any(|line| line == "Profiles:  1/1 ready (installed)"));
    assert!(lines
        .iter()
        .any(|line| line == "Manifest:  package (/tmp/manifest.json)"));
    assert!(lines.iter().any(|line| line == "  assets:  2026.0609.1"));
    assert!(lines
        .iter()
        .any(|line| line == "  - code: Code (ready, arch arm64, hash bbbbbbbbbbbb)"));
}

#[test]
fn health_issues_are_derived_from_profiles_status_payload() {
    let payload = serde_json::json!({
        "profile_count": 1,
        "profiles": [
            {
                "id": "code",
                "ready": false,
                "missing_assets": ["initrd.img"],
                "invalid_assets": ["rootfs.erofs"],
                "invalid_files": ["profiles/code/enforcement.toml"]
            }
        ]
    });

    let issues = profile_status_issues(&payload);

    assert_eq!(issues.len(), 1);
    assert!(issues[0].contains("Profile code is not ready"));
    assert!(issues[0].contains("missing assets: initrd.img"));
    assert!(issues[0].contains("invalid assets: rootfs.erofs"));
    assert!(issues[0].contains("invalid profile files: profiles/code/enforcement.toml"));
}

#[test]
fn parse_assets_ensure_json() {
    let cli = Cli::parse_from(["capsem", "assets", "ensure", "--json"]);
    match cli.command.unwrap() {
        Commands::Assets(AssetsCommands::Ensure { profile, json }) => {
            assert_eq!(profile, "code");
            assert!(json);
        }
        _ => panic!("expected assets ensure"),
    }
}

#[test]
fn parse_assets_status_profile() {
    let cli = Cli::parse_from(["capsem", "assets", "status", "--profile", "analysis"]);
    match cli.command.unwrap() {
        Commands::Assets(AssetsCommands::Status { profile, json }) => {
            assert_eq!(profile, "analysis");
            assert!(!json);
        }
        _ => panic!("expected assets status"),
    }
}

#[test]
fn parse_completions_bash() {
    let cli = Cli::parse_from(["capsem", "completions", "bash"]);
    assert!(matches!(
        cli.command.unwrap(),
        Commands::Misc(MiscCommands::Completions {
            shell: clap_complete::Shell::Bash
        })
    ));
}

#[test]
fn parse_uninstall() {
    let cli = Cli::parse_from(["capsem", "uninstall"]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Uninstall { yes }) => assert!(!yes),
        _ => panic!("expected Uninstall"),
    }
}

#[test]
fn parse_uninstall_yes() {
    let cli = Cli::parse_from(["capsem", "uninstall", "--yes"]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Uninstall { yes }) => assert!(yes),
        _ => panic!("expected Uninstall"),
    }
}

#[test]
fn parse_update() {
    let cli = Cli::parse_from(["capsem", "update"]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Update {
            yes,
            check,
            assets,
            channel,
            manifest,
            install_manifest_stdin,
            corp,
        }) => {
            assert!(!yes);
            assert!(!check);
            assert!(!assets);
            assert_eq!(channel, None);
            assert_eq!(manifest, None);
            assert!(!install_manifest_stdin);
            assert_eq!(corp, None);
        }
        _ => panic!("expected Update"),
    }
}

#[test]
fn parse_update_yes() {
    let cli = Cli::parse_from(["capsem", "update", "--yes"]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Update {
            yes,
            check,
            assets,
            channel,
            manifest,
            install_manifest_stdin,
            corp,
        }) => {
            assert!(yes);
            assert!(!check);
            assert!(!assets);
            assert_eq!(channel, None);
            assert_eq!(manifest, None);
            assert!(!install_manifest_stdin);
            assert_eq!(corp, None);
        }
        _ => panic!("expected Update"),
    }
}

#[test]
fn parse_update_check() {
    let cli = Cli::parse_from(["capsem", "update", "--check"]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Update {
            yes,
            check,
            assets,
            channel,
            manifest,
            install_manifest_stdin,
            corp,
        }) => {
            assert!(!yes);
            assert!(check);
            assert!(!assets);
            assert_eq!(channel, None);
            assert_eq!(manifest, None);
            assert!(!install_manifest_stdin);
            assert_eq!(corp, None);
        }
        _ => panic!("expected Update"),
    }
}

#[test]
fn parse_update_check_rejects_mutating_options() {
    for args in [
        vec!["capsem", "update", "--check", "--yes"],
        vec!["capsem", "update", "--check", "--assets"],
        vec![
            "capsem",
            "update",
            "--check",
            "--manifest",
            "https://release.capsem.org/assets/stable/manifest.json",
        ],
        vec![
            "capsem",
            "update",
            "--check",
            "--corp",
            "https://corp.example/capsem/corp.json",
        ],
    ] {
        assert!(
            Cli::try_parse_from(args.clone()).is_err(),
            "expected {args:?} to be rejected"
        );
    }
}

#[test]
fn parse_hidden_install_manifest_stdin_requires_the_exact_asset_handoff_shape() {
    let cli = Cli::parse_from([
        "capsem",
        "update",
        "--assets",
        "--manifest",
        "https://release.capsem.org/assets/nightly/manifest.json",
        "--install-manifest-stdin",
    ]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Update {
            assets,
            manifest,
            install_manifest_stdin,
            ..
        }) => {
            assert!(assets);
            assert!(manifest.is_some());
            assert!(install_manifest_stdin);
        }
        _ => panic!("expected Update"),
    }

    for args in [
        vec!["capsem", "update", "--install-manifest-stdin"],
        vec![
            "capsem",
            "update",
            "--manifest",
            "file:///tmp/manifest.json",
            "--install-manifest-stdin",
        ],
        vec![
            "capsem",
            "update",
            "--assets",
            "--manifest",
            "file:///tmp/manifest.json",
            "--install-manifest-stdin",
            "--corp",
            "file:///tmp/corp.toml",
        ],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
    let help = match Cli::try_parse_from(["capsem", "update", "--help"]) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("--help must stop parsing"),
    };
    assert!(!help.contains("install-manifest-stdin"));
}

#[test]
fn parse_update_assets() {
    let cli = Cli::parse_from(["capsem", "update", "--assets"]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Update {
            yes,
            check,
            assets,
            channel,
            manifest,
            install_manifest_stdin,
            corp,
        }) => {
            assert!(!yes);
            assert!(!check);
            assert!(assets);
            assert_eq!(channel, None);
            assert_eq!(manifest, None);
            assert!(!install_manifest_stdin);
            assert_eq!(corp, None);
        }
        _ => panic!("expected Update"),
    }
}

#[test]
fn parse_update_named_channel_for_check_and_asset_switch() {
    for args in [
        ["capsem", "update", "--check", "--channel", "nightly"],
        ["capsem", "update", "--assets", "--channel", "stable"],
    ] {
        let cli = Cli::parse_from(args);
        match cli.command.unwrap() {
            Commands::Misc(MiscCommands::Update { channel, .. }) => {
                assert!(matches!(channel.as_deref(), Some("nightly" | "stable")));
            }
            _ => panic!("expected Update"),
        }
    }
}

#[test]
fn parse_update_rejects_invalid_or_ambiguous_channel_selection() {
    for args in [
        vec!["capsem", "update", "--channel", "../nightly"],
        vec![
            "capsem",
            "update",
            "--channel",
            "nightly",
            "--manifest",
            "https://release.capsem.org/assets/stable/manifest.json",
        ],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
}

#[test]
fn parse_update_rejects_assets_with_corp_policy() {
    let err = match Cli::try_parse_from([
        "capsem",
        "update",
        "--assets",
        "--corp",
        "https://corp.example/capsem/corp.toml",
    ]) {
        Ok(_) => panic!("--corp provisions policy config and must not combine with --assets"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(message.contains("cannot be used with"), "{message}");
    assert!(message.contains("--assets"), "{message}");
}

#[test]
fn parse_update_url_overrides_reject_bare_paths() {
    for flag in ["--manifest", "--corp"] {
        for source in ["/tmp/capsem/manifest.json", "assets/stable/manifest.json"] {
            let err = match Cli::try_parse_from(["capsem", "update", "--assets", flag, source])
            {
                Ok(_) => panic!("update source overrides must reject bare filesystem paths"),
                Err(err) => err,
            };
            let message = err.to_string();
            assert!(
                message.contains(&format!("{flag} must be a URL")),
                "{message}"
            );
            assert!(message.contains("https://..."), "{message}");
            assert!(message.contains("http://..."), "{message}");
            assert!(message.contains("file:///absolute/path"), "{message}");
        }
    }
}

#[test]
fn parse_update_url_overrides_reject_url_shorthand_paths() {
    for flag in ["--manifest", "--corp"] {
        for (source, expected) in [
            (
                "file:assets/stable/manifest.json",
                "file URL must start with file://",
            ),
            (
                "https:release.capsem.org/assets/stable/manifest.json",
                "must use https://, http://, or file:// URLs",
            ),
        ] {
            let err = match Cli::try_parse_from(["capsem", "update", "--assets", flag, source])
            {
                Ok(_) => panic!("update source overrides must reject URL shorthand paths"),
                Err(err) => err,
            };
            let message = err.to_string();
            assert!(message.contains(expected), "{message}");
        }
    }
}

// -----------------------------------------------------------------------
// CAPSEM_RUN_DIR resolution
// -----------------------------------------------------------------------

#[test]
fn run_dir_override_logic() {
    let resolve = |env_val: Option<&str>, home: &str| -> PathBuf {
        env_val
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".capsem").join("run"))
    };
    assert_eq!(
        resolve(Some("/tmp/custom-run"), "/ignored"),
        PathBuf::from("/tmp/custom-run"),
    );
    assert_eq!(
        resolve(None, "/Users/test"),
        PathBuf::from("/Users/test/.capsem/run"),
    );
}

// -----------------------------------------------------------------------
// Fork / Image CLI parsing
// -----------------------------------------------------------------------

#[test]
fn parse_fork() {
    let cli = Cli::parse_from(["capsem", "fork", "my-vm", "my-image"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Fork {
            session,
            name,
            description,
        }) => {
            assert_eq!(session, "my-vm");
            assert_eq!(name, "my-image");
            assert_eq!(description, None);
        }
        _ => panic!("expected Fork"),
    }
}

#[test]
fn parse_fork_with_description() {
    let cli = Cli::parse_from(["capsem", "fork", "vm1", "img1", "-d", "My description"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Fork {
            session,
            name,
            description,
        }) => {
            assert_eq!(session, "vm1");
            assert_eq!(name, "img1");
            assert_eq!(description, Some("My description".into()));
        }
        _ => panic!("expected Fork"),
    }
}

#[test]
fn parse_create_with_from() {
    let cli = Cli::parse_from(["capsem", "create", "--from", "base-session"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { from, name, .. }) => {
            assert_eq!(from, Some("base-session".into()));
            assert_eq!(name, None);
        }
        _ => panic!("expected Create with --from"),
    }
}

#[test]
fn parse_create_with_from_image_alias() {
    // --image is a backward-compat alias for --from
    let cli = Cli::parse_from(["capsem", "create", "--image", "old-img"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { from, .. }) => {
            assert_eq!(from, Some("old-img".into()));
        }
        _ => panic!("expected Create with --image alias"),
    }
}

#[test]
fn parse_create_with_name_and_from() {
    let cli = Cli::parse_from(["capsem", "create", "-n", "my-session", "--from", "my-src"]);
    match cli.command.unwrap() {
        Commands::Session(SessionCommands::Create { name, from, .. }) => {
            assert_eq!(name, Some("my-session".into()));
            assert_eq!(from, Some("my-src".into()));
        }
        _ => panic!("expected Create with name and --from"),
    }
}

#[test]
fn shell_without_session_launches_tui_home() {
    assert_eq!(
        capsem_shell_tui_args(None, "http://127.0.0.1:49152"),
        vec![
            "--gateway-url".to_string(),
            "http://127.0.0.1:49152".to_string()
        ]
    );
}

#[test]
fn shell_with_session_focuses_tui_session() {
    assert_eq!(
        capsem_shell_tui_args(Some("profile-v2"), "http://127.0.0.1:49152"),
        vec![
            "--gateway-url".to_string(),
            "http://127.0.0.1:49152".to_string(),
            "--session".to_string(),
            "profile-v2".to_string()
        ]
    );
}

#[test]
fn gateway_runtime_url_rejects_cross_instance_fallbacks() {
    let run_dir = tempfile::tempdir().unwrap();
    assert!(gateway_url_from_runtime(run_dir.path()).is_none());

    std::fs::write(run_dir.path().join("gateway.port"), "not-a-port\n").unwrap();
    assert!(gateway_url_from_runtime(run_dir.path()).unwrap().is_err());

    std::fs::write(run_dir.path().join("gateway.port"), "0\n").unwrap();
    assert!(gateway_url_from_runtime(run_dir.path()).unwrap().is_err());

    std::fs::write(run_dir.path().join("gateway.port"), "49152\n").unwrap();
    assert_eq!(
        gateway_url_from_runtime(run_dir.path()).unwrap().unwrap(),
        "http://127.0.0.1:49152"
    );
}
