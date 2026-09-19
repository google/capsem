pub(super) fn gateway_status_body() -> &'static str {
    r#"{
        "service": "running",
        "gateway_version": "test",
        "vm_count": 2,
        "resource_summary": null,
        "vms": [
            {
                "id": "vm-1",
                "name": "profile-main",
                "status": "Running",
                "persistent": true,
                "profile_id": "profile-v2",
                "available_actions": ["stop"],
                "uptime_secs": 2840,
                "total_input_tokens": 30000,
                "total_output_tokens": 8912,
                "total_estimated_cost": 0.215,
                "total_tool_calls": 7,
                "total_requests": 11,
                "total_file_events": 3
            },
            {
                "id": "vm-2",
                "status": "Suspended",
                "persistent": true,
                "profile_id": "linux-os",
                "available_actions": [],
                "resume_blocked_reason": "profile payload hash drift",
                "uptime_secs": 7860,
                "total_input_tokens": 10000,
                "total_output_tokens": 2900,
                "total_estimated_cost": 0.076,
                "denied_requests": 1
            }
        ]
    }"#
}

pub(super) fn gateway_empty_status_body() -> &'static str {
    r#"{
        "service": "running",
        "gateway_version": "test",
        "vm_count": 0,
        "resource_summary": null,
        "vms": []
    }"#
}

pub(super) fn gateway_update_status_body() -> &'static str {
    r#"{
        "supply_chain": {"manifest":{"path":""},"channel_index":{},"host_sbom":{"name":""},"vm_obom":{"name":""},"attestations":[]},
        "checked_at": 1718444400,
        "channel_url": "https://release.capsem.org/health.json",
        "stale": false,
        "binary": {
            "current": "1.4.0",
            "latest": "1.4.1",
            "update_available": true,
            "state": "update_available",
            "compatibility": "compatible"
        },
        "assets": {
            "current": "assets-1",
            "latest": "assets-2",
            "update_available": true,
            "state": "update_available",
            "compatibility": "compatible"
        },
        "profiles": {
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        },
        "images": {
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        }
    }"#
}

pub(super) fn gateway_update_current_status_body() -> &'static str {
    r#"{
        "supply_chain": {"manifest":{"path":""},"channel_index":{},"host_sbom":{"name":""},"vm_obom":{"name":""},"attestations":[]},
        "checked_at": 1718444400,
        "channel_url": "https://release.capsem.org/health.json",
        "stale": false,
        "binary": {
            "current": "1.4.0",
            "latest": "1.4.0",
            "update_available": false,
            "state": "current",
            "compatibility": "compatible"
        },
        "assets": {
            "current": "assets-1",
            "latest": "assets-1",
            "update_available": false,
            "state": "current",
            "compatibility": "compatible"
        },
        "profiles": {
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        },
        "images": {
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        }
    }"#
}

pub(super) fn gateway_update_blocked_profile_status_body() -> &'static str {
    r#"{
        "supply_chain": {"manifest":{"path":""},"channel_index":{},"host_sbom":{"name":""},"vm_obom":{"name":""},"attestations":[]},
        "checked_at": 1718444400,
        "channel_url": "https://release.capsem.org/health.json",
        "stale": false,
        "binary": {
            "current": "1.4.0",
            "latest": "1.4.0",
            "update_available": false,
            "state": "current",
            "compatibility": "compatible"
        },
        "assets": {
            "current": "assets-1",
            "latest": "assets-1",
            "update_available": false,
            "state": "current",
            "compatibility": "compatible"
        },
        "profiles": {
            "current": "profiles-2030.0101.0",
            "latest": "profiles-2030.0101.1",
            "update_available": false,
            "state": "current",
            "compatibility": "compatible",
            "blocked_reason": "requires binary 1.4.1 or newer"
        },
        "images": {
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        }
    }"#
}

pub(super) fn gateway_update_blocked_asset_status_body() -> &'static str {
    r#"{
        "supply_chain": {"manifest":{"path":""},"channel_index":{},"host_sbom":{"name":""},"vm_obom":{"name":""},"attestations":[]},
        "checked_at": 1718444400,
        "channel_url": "https://release.capsem.org/health.json",
        "stale": false,
        "binary": {
            "current": "1.4.0",
            "latest": "1.4.0",
            "update_available": false,
            "state": "current",
            "compatibility": "compatible"
        },
        "assets": {
            "current": "2026.0627.1",
            "latest": "2030.0101.1",
            "update_available": false,
            "state": "unknown",
            "compatibility": "unknown",
            "blocked_reason": "requires binary 99.99.99 or newer"
        },
        "profiles": {
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        },
        "images": {
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        }
    }"#
}

pub(super) fn gateway_update_binary_with_blocked_profile_status_body() -> &'static str {
    r#"{
        "supply_chain": {"manifest":{"path":""},"channel_index":{},"host_sbom":{"name":""},"vm_obom":{"name":""},"attestations":[]},
        "checked_at": 1718444400,
        "channel_url": "https://release.capsem.org/health.json",
        "stale": false,
        "binary": {
            "current": "1.4.0",
            "latest": "1.4.1",
            "update_available": true,
            "state": "update_available",
            "compatibility": "compatible"
        },
        "assets": {
            "current": "assets-1",
            "latest": "assets-1",
            "update_available": false,
            "state": "current",
            "compatibility": "compatible"
        },
        "profiles": {
            "current": "profiles-2030.0101.0",
            "latest": "profiles-2030.0101.1",
            "update_available": false,
            "state": "current",
            "compatibility": "compatible",
            "blocked_reason": "requires binary 1.4.1 or newer"
        },
        "images": {
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        }
    }"#
}

pub(super) fn gateway_update_matrix_body(
    binary_update: bool,
    asset_update: bool,
    profile_update: bool,
    last_error: Option<&str>,
) -> String {
    let binary_latest = if binary_update { "1.4.1" } else { "1.4.0" };
    let asset_latest = if asset_update { "assets-2" } else { "assets-1" };
    let profile_latest = if profile_update {
        "profiles-2030.0101.1"
    } else {
        "profiles-2030.0101.0"
    };
    let error_field = last_error
        .map(|error| format!(r#","last_error":"{error}""#))
        .unwrap_or_default();
    format!(
        r#"{{
        "supply_chain": {{"manifest":{{"path":""}},"channel_index":{{}},"host_sbom":{{"name":""}},"vm_obom":{{"name":""}},"attestations":[]}},
        "checked_at": 1718444400,
        "channel_url": "https://release.capsem.org/health.json",
        "stale": false{error_field},
        "binary": {{
            "current": "1.4.0",
            "latest": "{binary_latest}",
            "update_available": {binary_update},
            "state": "current",
            "compatibility": "compatible"
        }},
        "assets": {{
            "current": "assets-1",
            "latest": "{asset_latest}",
            "update_available": {asset_update},
            "state": "current",
            "compatibility": "compatible"
        }},
        "profiles": {{
            "current": "profiles-2030.0101.0",
            "latest": "{profile_latest}",
            "update_available": {profile_update},
            "state": "current",
            "compatibility": "compatible"
        }},
        "images": {{
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        }}
    }}"#
    )
}

pub(super) fn gateway_profiles_body() -> &'static str {
    r#"{
        "profiles": [
            {
                "id": "code",
                "name": "Code",
                "description": "Optimized for coding and long-running agents.",
                "availability": { "web": true, "shell": true, "mobile": false },
                "update_semantics": {"new_sessions":"use_current_profile_catalog","existing_vms":"pinned_until_recreate","upgrade_action":"recreate_vm"},
                "source": "profile",
                "rule_count": 3,
                "default_rule_count": 2,
                "plugin_count": 1,
                "mcp_server_count": 1
            },
            {
                "id": "co-work",
                "name": "Co-work",
                "description": "Shared profile for collaborative agent sessions.",
                "availability": { "web": true, "shell": true, "mobile": false },
                "update_semantics": {"new_sessions":"use_current_profile_catalog","existing_vms":"pinned_until_recreate","upgrade_action":"recreate_vm"},
                "source": "profile",
                "rule_count": 4,
                "default_rule_count": 2,
                "plugin_count": 1,
                "mcp_server_count": 1
            }
        ]
    }"#
}

pub(super) fn gateway_profiles_with_unlaunchable_body() -> &'static str {
    r#"{
        "profiles": [
            {
                "id": "code",
                "name": "Code",
                "description": "Optimized for coding and long-running agents.",
                "availability": { "web": true, "shell": true, "mobile": false },
                "update_semantics": {"new_sessions":"use_current_profile_catalog","existing_vms":"pinned_until_recreate","upgrade_action":"recreate_vm"},
                "source": "profile",
                "rule_count": 3,
                "default_rule_count": 2,
                "plugin_count": 1,
                "mcp_server_count": 1
            },
            {
                "id": "web-only",
                "name": "Web Only",
                "description": "browser-only workflow",
                "availability": { "web": true, "shell": false, "mobile": false },
                "update_semantics": {"new_sessions":"use_current_profile_catalog","existing_vms":"pinned_until_recreate","upgrade_action":"recreate_vm"},
                "source": "corp",
                "rule_count": 1,
                "default_rule_count": 1,
                "plugin_count": 0,
                "mcp_server_count": 0
            },
            {
                "id": "mobile-only",
                "name": "Mobile Only",
                "description": "mobile-only workflow",
                "availability": { "web": false, "shell": false, "mobile": true },
                "update_semantics": {"new_sessions":"use_current_profile_catalog","existing_vms":"pinned_until_recreate","upgrade_action":"recreate_vm"},
                "source": "corp",
                "rule_count": 1,
                "default_rule_count": 1,
                "plugin_count": 0,
                "mcp_server_count": 0
            }
        ]
    }"#
}
