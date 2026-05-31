use std::collections::HashMap;

use capsem_core::settings_profiles::{self, CapabilityMode, VmNetworkMode};
use capsem_core::vm::guest_config::{GuestConfig, GuestFile};

fn network_defaults_from_effective(
    effective: Option<&settings_profiles::EffectiveVmSettings>,
) -> (bool, bool) {
    if matches!(
        effective.map(|effective| effective.vm.value.network),
        Some(VmNetworkMode::Disabled)
    ) {
        return (false, false);
    }

    match effective
        .map(|effective| effective.security.value.capabilities.network_egress)
        .unwrap_or(CapabilityMode::Ask)
    {
        CapabilityMode::Allow | CapabilityMode::Audit => (true, true),
        CapabilityMode::Ask => (true, true),
        CapabilityMode::Block => (false, false),
    }
}

pub(super) fn guest_config_from_effective(
    effective: Option<&settings_profiles::EffectiveVmSettings>,
) -> GuestConfig {
    let (default_allow_read, default_allow_write) = network_defaults_from_effective(effective);

    let provider_allowed = |name: &str| {
        effective
            .and_then(|effective| effective.ai.value.providers.get(name))
            .map(|provider| provider.enabled)
            .unwrap_or(default_allow_read)
    };

    let mut env = HashMap::new();
    env.insert(
        "REQUESTS_CA_BUNDLE".to_string(),
        "/etc/ssl/certs/ca-certificates.crt".to_string(),
    );
    env.insert(
        "NODE_EXTRA_CA_CERTS".to_string(),
        "/etc/ssl/certs/ca-certificates.crt".to_string(),
    );
    env.insert(
        "SSL_CERT_FILE".to_string(),
        "/etc/ssl/certs/ca-certificates.crt".to_string(),
    );
    env.insert("TERM".to_string(), "xterm-256color".to_string());
    env.insert("HOME".to_string(), "/root".to_string());
    env.insert(
        "PATH".to_string(),
        "/var/lib/capsem/venv/bin:/root/.local/bin:/opt/ai-clis/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
    );
    env.insert(
        "VIRTUAL_ENV".to_string(),
        "/var/lib/capsem/venv".to_string(),
    );
    env.insert(
        "UV_CACHE_DIR".to_string(),
        "/var/cache/capsem/uv".to_string(),
    );
    env.insert("LANG".to_string(), "C".to_string());
    env.insert(
        "CAPSEM_WEB_ALLOW_READ".to_string(),
        if default_allow_read { "1" } else { "0" }.to_string(),
    );
    env.insert(
        "CAPSEM_WEB_ALLOW_WRITE".to_string(),
        if default_allow_write { "1" } else { "0" }.to_string(),
    );
    env.insert(
        "CAPSEM_OPENAI_ALLOWED".to_string(),
        if provider_allowed("openai") { "1" } else { "0" }.to_string(),
    );
    env.insert(
        "CAPSEM_ANTHROPIC_ALLOWED".to_string(),
        if provider_allowed("anthropic") {
            "1"
        } else {
            "0"
        }
        .to_string(),
    );
    env.insert(
        "CAPSEM_GOOGLE_ALLOWED".to_string(),
        if provider_allowed("google") { "1" } else { "0" }.to_string(),
    );
    if let Some(effective) = effective {
        for (key, value) in &effective.credential_env {
            env.insert(key.clone(), value.clone());
        }
    }

    let files = vec![
        GuestFile {
            path: "/root/.local/bin/gemini".to_string(),
            content: r#"#!/bin/sh
for arg in "$@"; do
  case "$arg" in
    --yolo|-y|--help|-h|--version|version)
      exec /opt/ai-clis/bin/gemini "$@"
      ;;
  esac
done
exec /opt/ai-clis/bin/gemini --yolo "$@"
"#
            .to_string(),
            mode: 0o755,
        },
        GuestFile {
            path: "/root/.gemini/settings.json".to_string(),
            content: r#"{"homeDirectoryWarningDismissed":true,"general":{"disableAutoUpdate":true,"disableUpdateNag":true},"ui":{"hideTips":true,"hideBanner":false},"privacy":{"usageStatisticsEnabled":false,"sessionRetention":"none"},"telemetry":{"enabled":false},"security":{"auth":{"selectedType":"gemini-api-key"},"folderTrust.enabled":false},"ide":{"hasSeenNudge":true},"tools":{"sandbox":false},"mcpServers":{"local":{"command":"/run/capsem-mcp-server"}}}"#.to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.gemini/installation_id".to_string(),
            content: "capsem-sandbox-00000000-0000-0000-0000-000000000000".to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.gemini/projects.json".to_string(),
            content: r#"{"projects":{"/root":"root"}}"#.to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.gemini/trustedFolders.json".to_string(),
            content: r#"{"/root":"TRUST_FOLDER"}"#.to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.codex/config.toml".to_string(),
            content: "[mcp_servers.local]\ncommand = \"/run/capsem-mcp-server\"\n".to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.claude/settings.json".to_string(),
            content: r#"{"permissions":{"defaultMode":"bypassPermissions"},"env":{"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"},"mcpServers":{"local":{"command":"/run/capsem-mcp-server"}}}"#.to_string(),
            mode: 0o600,
        },
        GuestFile {
            path: "/root/.claude.json".to_string(),
            content: r#"{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true,"hasTrustDialogHooksAccepted":true,"shiftEnterKeyBindingInstalled":true,"theme":"dark","numStartups":1,"opusProMigrationComplete":true,"sonnet1m45MigrationComplete":true,"projects":{"/root":{"allowedTools":[],"hasTrustDialogAccepted":true,"projectOnboardingSeenCount":1}},"mcpServers":{"local":{"command":"/run/capsem-mcp-server"}}}"#.to_string(),
            mode: 0o600,
        },
    ];

    GuestConfig {
        env: Some(env),
        files: Some(files),
    }
}
