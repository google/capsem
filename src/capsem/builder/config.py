"""Config loader + defaults.json generator.

Loads TOML configs from guest/config/ into Pydantic models, and transforms
them into the defaults.json format consumed by Rust at compile time.
"""

from __future__ import annotations

import tomllib
from pathlib import Path
from typing import Any

from capsem.builder.models import (
    AiProviderConfig,
    BuildConfig,
    GuestImageConfig,
    ImageManifestConfig,
    McpServerConfig,
    PackageSetConfig,
    VmEnvironmentConfig,
    VmResourcesConfig,
    WebSecurityConfig,
    WebServiceConfig,
)


def parse_toml(path: Path) -> dict:
    """Read and parse a TOML file."""
    with open(path, "rb") as f:
        return tomllib.load(f)


def _load_build(config_dir: Path) -> BuildConfig:
    data = parse_toml(config_dir / "build.toml")
    return BuildConfig.model_validate(data["build"])


def _load_manifest(config_dir: Path) -> ImageManifestConfig | None:
    manifest_path = config_dir / "manifest.toml"
    if not manifest_path.is_file():
        return None
    data = parse_toml(manifest_path)
    return ImageManifestConfig.model_validate(data["image"])


def _load_ai_providers(config_dir: Path) -> dict[str, AiProviderConfig]:
    ai_dir = config_dir / "ai"
    providers: dict[str, AiProviderConfig] = {}
    if ai_dir.is_dir():
        for path in sorted(ai_dir.glob("*.toml")):
            data = parse_toml(path)
            for key, value in data.items():
                providers[key] = AiProviderConfig.model_validate(value)
    return providers


def _load_package_sets(config_dir: Path) -> dict[str, PackageSetConfig]:
    pkg_dir = config_dir / "packages"
    sets: dict[str, PackageSetConfig] = {}
    if pkg_dir.is_dir():
        for path in sorted(pkg_dir.glob("*.toml")):
            data = parse_toml(path)
            for key, value in data.items():
                sets[key] = PackageSetConfig.model_validate(value)
    return sets


def _load_mcp_servers(config_dir: Path) -> dict[str, McpServerConfig]:
    mcp_dir = config_dir / "mcp"
    servers: dict[str, McpServerConfig] = {}
    if mcp_dir.is_dir():
        for path in sorted(mcp_dir.glob("*.toml")):
            data = parse_toml(path)
            for key, value in data.items():
                servers[key] = McpServerConfig.model_validate(value)
    return servers


def _load_web_security(config_dir: Path) -> WebSecurityConfig:
    path = config_dir / "security" / "web.toml"
    if not path.exists():
        return WebSecurityConfig()
    data = parse_toml(path)
    return WebSecurityConfig.model_validate(data["web"])


def _load_vm_resources(config_dir: Path) -> VmResourcesConfig:
    path = config_dir / "vm" / "resources.toml"
    if not path.exists():
        return VmResourcesConfig()
    data = parse_toml(path)
    return VmResourcesConfig.model_validate(data["resources"])


def _load_vm_environment(config_dir: Path) -> VmEnvironmentConfig:
    path = config_dir / "vm" / "environment.toml"
    if not path.exists():
        return VmEnvironmentConfig()
    data = parse_toml(path)
    return VmEnvironmentConfig.model_validate(data["environment"])


def load_guest_config(guest_dir: Path) -> GuestImageConfig:
    """Walk a guest/config/ directory, parse all TOML files, return GuestImageConfig.

    Args:
        guest_dir: Path to the guest directory (contains config/ subdirectory).

    Returns:
        GuestImageConfig with all parsed and validated config.

    Raises:
        FileNotFoundError: If guest_dir/config/build.toml is missing (required).
        pydantic.ValidationError: If any TOML file fails validation.
    """
    config_dir = guest_dir / "config"
    return GuestImageConfig(
        build=_load_build(config_dir),
        manifest=_load_manifest(config_dir),
        ai_providers=_load_ai_providers(config_dir),
        package_sets=_load_package_sets(config_dir),
        mcp_servers=_load_mcp_servers(config_dir),
        web_security=_load_web_security(config_dir),
        vm_resources=_load_vm_resources(config_dir),
        vm_environment=_load_vm_environment(config_dir),
    )


# ---------------------------------------------------------------------------
# defaults.json generator
# ---------------------------------------------------------------------------

# Repository token metadata -- static data not in TOML configs.
_REPO_TOKEN_META: dict[str, dict[str, Any]] = {
    "github": {
        "name": "GitHub Token",
        "description": "Personal access token for git push over HTTPS. Injected into .git-credentials.",
        "env_vars": ["GH_TOKEN", "GITHUB_TOKEN"],
        "docs_url": "https://github.com/settings/tokens",
        "prefix": "ghp_",
    },
    "gitlab": {
        "name": "GitLab Token",
        "description": "Personal access token for git push over HTTPS. Injected into .git-credentials.",
        "env_vars": ["GITLAB_TOKEN"],
        "docs_url": "https://gitlab.com/-/user_settings/personal_access_tokens",
        "prefix": "glpat-",
    },
}


def _http_rules(allow_get: bool, allow_post: bool) -> dict:
    """Build meta.rules from get/post flags."""
    rule: dict[str, bool] = {}
    if allow_get:
        rule["get"] = True
    if allow_post:
        rule["post"] = True
    return {"default": rule} if rule else {}


def _ai_provider_section(key: str, prov: AiProviderConfig) -> dict:
    """Build the JSON object for one AI provider under settings.ai."""
    section: dict[str, Any] = {
        "name": prov.name,
        "description": prov.description,
        "enabled_by": f"ai.{key}.allow",
        "collapsed": False,
        "allow": {
            "name": f"Allow {prov.name}",
            "description": f"Enable API access to {prov.name} ({prov.network.domains[0]}).",
            "type": "bool",
            "default": prov.enabled,
            "meta": {"rules": _http_rules(prov.network.allow_get, prov.network.allow_post)},
        },
        "api_key": {
            "name": prov.api_key.name,
            "description": f"API key for {prov.name}. Injected as {prov.api_key.env_vars[0]} env var.",
            "type": "apikey",
            "default": "",
            "meta": {
                "env_vars": prov.api_key.env_vars,
                **({"docs_url": prov.api_key.docs_url} if prov.api_key.docs_url else {}),
                **({"prefix": prov.api_key.prefix} if prov.api_key.prefix else {}),
            },
        },
        "domains": {
            "name": f"{prov.name} Domains",
            "description": "Comma-separated domain patterns. Wildcards (*.example.com) match all subdomains.",
            "type": "text",
            "default": ", ".join(prov.network.domains),
        },
    }

    # CLI sub-group for files
    if prov.files and prov.cli:
        cli_group: dict[str, Any] = {
            "name": prov.cli.name,
            "description": prov.cli.description,
        }
        for file_key, file_cfg in prov.files.items():
            file_entry: dict[str, Any] = {
                "name": _file_display_name(prov.cli.name, file_key),
                "description": _file_description(prov.cli.name, file_key, file_cfg.path),
                "type": "file",
                "default": {"path": file_cfg.path, "content": file_cfg.content},
            }
            filetype = _infer_filetype(file_cfg.path)
            if filetype:
                file_entry["meta"] = {"filetype": filetype}
            cli_group[file_key] = file_entry
        section[prov.cli.key] = cli_group

    return section


def _file_display_name(cli_name: str, file_key: str) -> str:
    """Derive display name for a file setting."""
    # Map common file keys to human-readable names
    key_names = {
        "settings_json": f"{cli_name} settings.json",
        "state_json": f"{cli_name} state (.claude.json)",
        "credentials_json": f"{cli_name} OAuth credentials",
        "config_toml": f"{cli_name} config.toml",
        "projects_json": f"{cli_name} projects.json",
        "trusted_folders_json": f"{cli_name} trustedFolders.json",
        "installation_id": f"{cli_name} installation_id",
        "google_adc_json": "Google Cloud ADC",
    }
    return key_names.get(file_key, f"{cli_name} {file_key}")


def _file_description(cli_name: str, file_key: str, path: str) -> str:
    """Derive description for a file setting."""
    descs = {
        "settings_json": f"Content for {path}. Bypass permissions, disable telemetry/updates for sandboxed execution.",
        "state_json": f"Content for {path}. Skips onboarding, trust dialogs, and keybinding prompts.",
        "credentials_json": f"Content for {path}. OAuth tokens for subscription-based auth (Pro/Max). Injected from host when detected.",
        "config_toml": f"Content for {path}. MCP servers, auth, etc.",
        "projects_json": f"Content for {path}. Project directory mappings.",
        "trusted_folders_json": f"Content for {path}. Pre-trusted workspace dirs.",
        "installation_id": f"Content for {path}. Stable UUID avoids first-run prompts.",
        "google_adc_json": f"Content for {path}. OAuth credentials for Google Cloud auth. Injected from host when detected.",
    }
    return descs.get(file_key, f"Content for {path}.")


def _infer_filetype(path: str) -> str | None:
    """Infer filetype from file extension."""
    if path.endswith(".json"):
        return "json"
    if path.endswith(".toml"):
        return "toml"
    if path.endswith(".conf"):
        return "conf"
    if path.endswith(".bashrc") or path.endswith(".bash"):
        return "bash"
    return None


def _web_service_entry(
    key: str, svc: WebServiceConfig, prefix: str
) -> dict[str, Any]:
    """Build JSON object for a search engine, registry, or repo provider."""
    return {
        "name": svc.name,
        "description": f"{svc.name} {'web search' if 'search' in prefix else ('package registry' if 'registry' in prefix else 'and ' + svc.name + '-hosted content')}",
        "enabled_by": f"{prefix}.{key}.allow",
        "allow": {
            "name": f"Allow {svc.name}",
            "description": f"Enable access to {svc.name}{' web search' if 'search' in prefix else (' and ' + svc.name + '-hosted content' if 'repository' in prefix else '')}.",
            "type": "bool",
            "default": svc.enabled,
            "meta": {
                "domains": svc.domains,
                "rules": _http_rules(svc.allow_get, svc.allow_post),
            },
        },
        "domains": {
            "name": f"{svc.name} Domains",
            "description": "Comma-separated domain patterns. Wildcards (*.example.com) match all subdomains.",
            "type": "text",
            "default": ", ".join(svc.domains),
            "meta": {"format": "domain_list"},
        },
    }


def _repo_provider_entry(
    key: str, svc: WebServiceConfig
) -> dict[str, Any]:
    """Build JSON object for a repository provider (GitHub, GitLab)."""
    entry = _web_service_entry(key, svc, "repository.providers")
    # Repository providers also have a token setting
    if key in _REPO_TOKEN_META:
        meta = _REPO_TOKEN_META[key]
        entry["token"] = {
            "name": meta["name"],
            "description": meta["description"],
            "type": "apikey",
            "default": "",
            "meta": {
                "env_vars": meta["env_vars"],
                "docs_url": meta["docs_url"],
                "prefix": meta["prefix"],
            },
        }
    return entry


def generate_defaults_json(config: GuestImageConfig) -> dict:
    """Transform GuestImageConfig into the defaults.json dict.

    Produces the hierarchical JSON consumed by Rust's registry.rs at compile time.
    Combines data from TOML configs with hardcoded host-only settings.
    """
    settings: dict[str, Any] = {}

    # -- app (host-only) --
    settings["app"] = {
        "name": "App",
        "description": "Application settings",
        "collapsed": False,
        "auto_update": {
            "name": "Auto-check for updates",
            "description": "Check for new Capsem versions on launch",
            "type": "bool",
            "default": True,
        },
        "check_update": {
            "name": "Check for updates",
            "description": "Manually check if a new version is available",
            "action": "check_update",
        },
    }

    # -- ai (from TOML configs) --
    ai_section: dict[str, Any] = {
        "name": "AI Providers",
        "description": "AI model provider configuration",
        "collapsed": False,
    }
    for key, prov in config.ai_providers.items():
        ai_section[key] = _ai_provider_section(key, prov)
    settings["ai"] = ai_section

    # -- repository (git identity host-only + providers from web.toml) --
    repo_provs: dict[str, Any] = {
        "name": "Providers",
        "description": "Code hosting platforms",
    }
    for key, svc in config.web_security.repository.items():
        repo_provs[key] = _repo_provider_entry(key, svc)
    settings["repository"] = {
        "name": "Repositories",
        "description": "Code hosting and git configuration",
        "collapsed": False,
        "git": {
            "identity": {
                "name": "Git Identity",
                "description": "Author name and email for commits inside the VM",
                "author_name": {
                    "name": "Author name",
                    "description": "Name used for git commits. Injected as GIT_AUTHOR_NAME and GIT_COMMITTER_NAME.",
                    "type": "text",
                    "default": "",
                    "meta": {"env_vars": ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"]},
                },
                "author_email": {
                    "name": "Author email",
                    "description": "Email used for git commits. Injected as GIT_AUTHOR_EMAIL and GIT_COMMITTER_EMAIL.",
                    "type": "text",
                    "default": "",
                    "meta": {"env_vars": ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL"]},
                },
            },
        },
        "providers": repo_provs,
    }

    # -- security (preset action + web defaults + services from web.toml) --
    search_section: dict[str, Any] = {
        "name": "Search Engines",
        "description": "Web search engine access",
    }
    for key, svc in config.web_security.search.items():
        search_section[key] = _web_service_entry(
            key, svc, "security.services.search"
        )

    registry_section: dict[str, Any] = {
        "name": "Package Registries",
        "description": "Package manager registries",
    }
    for key, svc in config.web_security.registry.items():
        registry_section[key] = _web_service_entry(
            key, svc, "security.services.registry"
        )

    ws = config.web_security
    settings["security"] = {
        "name": "Security",
        "description": "Network access control, web services, and security presets",
        "collapsed": False,
        "preset": {
            "name": "Security Preset",
            "description": "Predefined security configurations",
            "action": "preset_select",
        },
        "web": {
            "name": "Web",
            "description": "Default actions for unknown domains",
            "allow_read": {
                "name": "Allow read requests",
                "description": "Allow GET/HEAD/OPTIONS for domains not in any allow/block list.",
                "type": "bool",
                "default": ws.allow_read,
            },
            "allow_write": {
                "name": "Allow write requests",
                "description": "Allow POST/PUT/DELETE/PATCH for domains not in any allow/block list.",
                "type": "bool",
                "default": ws.allow_write,
            },
            "custom_allow": {
                "name": "Allowed domains",
                "description": "Comma-separated domain patterns to allow. Wildcards supported (*.example.com).",
                "type": "text",
                "default": ", ".join(ws.custom_allow),
                "meta": {"format": "domain_list"},
            },
            "custom_block": {
                "name": "Blocked domains",
                "description": "Comma-separated domain patterns to block. Takes priority over custom allow list.",
                "type": "text",
                "default": ", ".join(ws.custom_block) if ws.custom_block else "",
                "meta": {"format": "domain_list"},
            },
        },
        "services": {
            "name": "Services",
            "description": "Search engines and package registries",
            "search": search_section,
            "registry": registry_section,
        },
    }

    # -- vm (actions + snapshots host-only + environment + resources from TOML) --
    env = config.vm_environment
    shell_section: dict[str, Any] = {
        "name": "Shell",
        "description": "Guest shell settings",
        "term": {
            "name": "TERM",
            "description": "Terminal type for the guest shell.",
            "type": "text",
            "default": env.shell.term,
            "meta": {"env_vars": ["TERM"]},
        },
        "home": {
            "name": "HOME",
            "description": "Home directory for the guest shell.",
            "type": "text",
            "default": env.shell.home,
            "meta": {"env_vars": ["HOME"]},
        },
        "path": {
            "name": "PATH",
            "description": "Executable search path for the guest shell.",
            "type": "text",
            "default": env.shell.path,
            "meta": {"env_vars": ["PATH"]},
        },
        "lang": {
            "name": "LANG",
            "description": "Locale for the guest shell.",
            "type": "text",
            "default": env.shell.lang,
            "meta": {"env_vars": ["LANG"]},
        },
    }
    if env.shell.bashrc:
        shell_section["bashrc"] = {
            "name": "Bash configuration",
            "description": "User shell config sourced at login. Customize prompt, aliases, and functions.",
            "type": "file",
            "default": {"path": env.shell.bashrc.path, "content": env.shell.bashrc.content},
            "meta": {"filetype": "bash"},
        }
    if env.shell.tmux_conf:
        shell_section["tmux_conf"] = {
            "name": "tmux configuration",
            "description": "tmux terminal multiplexer config. Customize appearance, keybindings, and behavior.",
            "type": "file",
            "default": {"path": env.shell.tmux_conf.path, "content": env.shell.tmux_conf.content},
            "meta": {"filetype": "conf"},
        }

    res = config.vm_resources
    settings["vm"] = {
        "name": "VM",
        "description": "Virtual machine configuration",
        "collapsed": False,
        "rerun_wizard": {
            "name": "Setup Wizard",
            "description": "Re-run the first-time setup wizard to reconfigure providers, repositories, and security.",
            "action": "rerun_wizard",
        },
        "snapshots": {
            "name": "Snapshots",
            "description": "Automatic and manual workspace snapshot settings",
            "auto_max": {
                "name": "Auto snapshot limit",
                "description": "Maximum number of automatic rolling snapshots.",
                "type": "number",
                "default": 10,
                "meta": {"min": 1, "max": 50},
            },
            "manual_max": {
                "name": "Manual snapshot limit",
                "description": "Maximum number of named manual snapshots.",
                "type": "number",
                "default": 12,
                "meta": {"min": 1, "max": 50},
            },
            "auto_interval": {
                "name": "Auto snapshot interval",
                "description": "Seconds between automatic snapshots.",
                "type": "number",
                "default": 300,
                "meta": {"min": 30, "max": 3600},
            },
        },
        "environment": {
            "name": "Environment",
            "description": "Shell and environment variables",
            "shell": shell_section,
            "ssh": {
                "name": "SSH",
                "description": "SSH key configuration",
                "public_key": {
                    "name": "SSH public key",
                    "description": "Public key injected as /root/.ssh/authorized_keys in the guest VM.",
                    "type": "text",
                    "default": "",
                },
            },
            "tls": {
                "name": "TLS",
                "description": "TLS certificate configuration",
                "ca_bundle": {
                    "name": "CA bundle path",
                    "description": "Path to the CA certificate bundle in the guest. Injected as REQUESTS_CA_BUNDLE, NODE_EXTRA_CA_CERTS, and SSL_CERT_FILE.",
                    "type": "text",
                    "default": env.tls.ca_bundle,
                    "meta": {"env_vars": ["REQUESTS_CA_BUNDLE", "NODE_EXTRA_CA_CERTS", "SSL_CERT_FILE"]},
                },
            },
        },
        "resources": {
            "name": "Resources",
            "description": "Hardware, telemetry, and session limits",
            "cpu_count": {"name": "CPU cores", "description": "Number of CPU cores allocated to the VM.", "type": "number", "default": res.cpu_count, "meta": {"min": 1, "max": 8}},
            "ram_gb": {"name": "RAM", "description": "Amount of RAM allocated to the VM in GB.", "type": "number", "default": res.ram_gb, "meta": {"min": 1, "max": 16}},
            "scratch_disk_size_gb": {"name": "Scratch disk size", "description": "Size of the ephemeral scratch disk in GB.", "type": "number", "default": res.scratch_disk_size_gb, "meta": {"min": 1, "max": 128}},
            "log_bodies": {"name": "Log request bodies", "description": "Capture request/response bodies in telemetry.", "type": "bool", "default": res.log_bodies},
            "max_body_capture": {"name": "Max body capture", "description": "Maximum bytes of body to capture in telemetry.", "type": "number", "default": res.max_body_capture, "meta": {"min": 0, "max": 1048576}},
            "retention_days": {"name": "Session retention", "description": "Number of days to retain session data.", "type": "number", "default": res.retention_days, "meta": {"min": 1, "max": 365}},
            "max_sessions": {"name": "Maximum sessions", "description": "Keep at most this many sessions (oldest culled first).", "type": "number", "default": res.max_sessions, "meta": {"min": 1, "max": 10000}},
            "min_content_sessions": {"name": "Minimum content sessions", "description": "Always keep at least this many sessions that contain AI activity, regardless of age. Empty test sessions are terminated first.", "type": "number", "default": res.min_content_sessions, "meta": {"min": 0, "max": 1000, "step": 1}},
            "max_disk_gb": {"name": "Maximum disk usage", "description": "Maximum total disk usage for all sessions in GB.", "type": "number", "default": res.max_disk_gb, "meta": {"min": 1, "max": 1000}},
            "terminated_retention_days": {"name": "Terminated session retention", "description": "Days to keep terminated session records in the index. After this, the record is permanently deleted.", "type": "number", "default": res.terminated_retention_days, "meta": {"min": 30, "max": 3650}},
        },
    }

    # -- appearance (host-only) --
    settings["appearance"] = {
        "name": "Appearance",
        "description": "UI appearance and display settings",
        "collapsed": False,
        "dark_mode": {
            "name": "Dark mode",
            "description": "Use dark color scheme in the UI.",
            "type": "bool",
            "default": True,
            "meta": {"side_effect": "toggle_theme"},
        },
        "font_size": {
            "name": "Font size",
            "description": "Terminal font size in pixels.",
            "type": "number",
            "default": 14,
            "meta": {"min": 8, "max": 32},
        },
    }

    # -- mcp (from TOML configs) --
    mcp: dict[str, Any] = {}
    for key, server in config.mcp_servers.items():
        entry: dict[str, Any] = {
            "name": server.name,
            "description": server.description,
            "transport": server.transport.value,
        }
        if server.command:
            entry["command"] = server.command
        if server.url:
            entry["url"] = server.url
        if server.args:
            entry["args"] = server.args
        if server.env:
            entry["env"] = server.env
        if server.headers:
            entry["headers"] = server.headers
        if server.builtin:
            entry["builtin"] = server.builtin
        mcp[key] = entry

    return {"settings": settings, "mcp": mcp}


# ---------------------------------------------------------------------------
# mock-settings.generated.ts generator
# ---------------------------------------------------------------------------


def _ts_value(val: Any, *, context: str = "") -> str:
    """Serialize a Python value to a TypeScript literal."""
    if val is None:
        return "null"
    if isinstance(val, bool):
        return "true" if val else "false"
    if isinstance(val, (int, float)):
        return str(val)
    if isinstance(val, str):
        # Escape backslashes, single quotes, and newlines for TS string
        escaped = val.replace("\\", "\\\\").replace("'", "\\'").replace("\n", "\\n")
        return f"'{escaped}'"
    if isinstance(val, dict):
        # HttpMethodPermissions rule objects need all required fields
        if context == "rules" and val:
            pairs = ", ".join(
                f"{k}: {_ts_http_rule(v)}" for k, v in val.items()
            )
            return f"{{ {pairs} }}"
        pairs = ", ".join(f"{k}: {_ts_value(v)}" for k, v in val.items())
        return f"{{ {pairs} }}"
    if isinstance(val, list):
        items = ", ".join(_ts_value(v) for v in val)
        return f"[{items}]"
    return repr(val)


def _ts_http_rule(rule: dict) -> str:
    """Serialize an HTTP method permission rule with all required fields."""
    return (
        "{ "
        f"domains: [], path: null, "
        f"get: {_ts_value(rule.get('get', False))}, "
        f"post: {_ts_value(rule.get('post', False))}, "
        f"put: {_ts_value(rule.get('put', False))}, "
        f"delete: {_ts_value(rule.get('delete', False))}, "
        f"other: {_ts_value(rule.get('other', False))}"
        " }"
    )


def _ts_meta(meta: dict) -> str:
    """Serialize a metadata dict, handling rules with full HttpMethodPermissions."""
    pairs = []
    for k, v in meta.items():
        if k == "rules":
            pairs.append(f"{k}: {_ts_value(v, context='rules')}")
        else:
            pairs.append(f"{k}: {_ts_value(v)}")
    return f"{{ {', '.join(pairs)} }}"


def _collect_mock_settings(
    table: dict, path: str, parent_category: str, parent_enabled_by: str | None,
) -> list[dict[str, Any]]:
    """Walk defaults.json hierarchy, collect leaf settings as mock entries."""
    # Skip action nodes
    if "action" in table:
        return []

    if "type" in table:
        # Leaf setting
        meta: dict[str, Any] = {
            "domains": [],
            "choices": [],
            "min": None,
            "max": None,
            "rules": {},
        }
        raw_meta = table.get("meta", {})
        if raw_meta.get("domains"):
            meta["domains"] = raw_meta["domains"]
        if raw_meta.get("choices"):
            meta["choices"] = raw_meta["choices"]
        if raw_meta.get("min") is not None:
            meta["min"] = raw_meta["min"]
        if raw_meta.get("max") is not None:
            meta["max"] = raw_meta["max"]
        if raw_meta.get("rules"):
            meta["rules"] = raw_meta["rules"]
        # Optional metadata fields
        for key in ("docs_url", "prefix", "filetype", "format", "widget",
                    "side_effect", "step"):
            if raw_meta.get(key) is not None:
                meta[key] = raw_meta[key]

        enabled_by = parent_enabled_by
        # If this IS the toggle itself, don't set enabled_by on it
        if enabled_by == path:
            enabled_by = None

        default_val = table["default"]
        setting_type = table["type"]
        enabled = True
        if enabled_by is not None:
            # Default to disabled (parent toggle is typically false by default)
            enabled = False

        entry: dict[str, Any] = {
            "id": path,
            "category": parent_category,
            "name": table["name"],
            "setting_type": setting_type,
            "description": table.get("description", ""),
            "default_value": default_val,
            "effective_value": default_val,
            "enabled_by": enabled_by,
            "enabled": enabled,
            "metadata": meta,
        }
        return [entry]

    # Group node -- recurse into children
    category = table.get("name", parent_category)
    group_enabled_by = table.get("enabled_by", parent_enabled_by)

    results: list[dict[str, Any]] = []
    for key, val in table.items():
        if key in ("name", "description", "enabled_by", "collapsed"):
            continue
        if isinstance(val, dict):
            child_path = f"{path}.{key}" if path else key
            results.extend(
                _collect_mock_settings(val, child_path, category, group_enabled_by)
            )
    return results


def _build_mock_tree_ts(
    table: dict, path: str, parent_enabled_by: str | None, indent: int,
) -> list[str]:
    """Walk defaults.json hierarchy, produce TypeScript tree node lines."""
    pad = "  " * indent

    # Action node
    if "action" in table:
        name = table.get("name", "")
        desc = table.get("description", "")
        action = table["action"]
        return [
            f"{pad}{{ kind: 'action', key: {_ts_value(path)}, "
            f"name: {_ts_value(name)}, description: {_ts_value(desc)}, "
            f"action: {_ts_value(action)} }} as any,"
        ]

    # Leaf setting
    if "type" in table:
        return [
            f"{pad}leaf(mockSettings.find(s => s.id === {_ts_value(path)})!),"
        ]

    # Group node
    group_name = table.get("name")
    group_desc = table.get("description", "")
    group_enabled_by = table.get("enabled_by", parent_enabled_by)
    group_collapsed = table.get("collapsed", False)

    # Collect children
    child_lines: list[str] = []
    for key, val in table.items():
        if key in ("name", "description", "enabled_by", "collapsed"):
            continue
        if isinstance(val, dict):
            child_path = f"{path}.{key}" if path else key
            child_lines.extend(
                _build_mock_tree_ts(val, child_path, group_enabled_by, indent + 1)
            )

    if not child_lines:
        return []

    # Top-level call (no path) returns children directly
    if not path:
        return child_lines

    if group_name:
        lines = []
        eb = ""
        if group_enabled_by and group_enabled_by != parent_enabled_by:
            eb = f" enabled_by: {_ts_value(group_enabled_by)},"
        lines.append(
            f"{pad}{{"
            f" kind: 'group', enabled: true, key: {_ts_value(path)},"
            f" name: {_ts_value(group_name)},"
            f" description: {_ts_value(group_desc)},"
            f"{eb}"
            f" collapsed: {_ts_value(group_collapsed)}, children: ["
        )
        lines.extend(child_lines)
        lines.append(f"{pad}]}},")
        return lines

    # Unnamed group -- inline children
    return child_lines


def generate_mock_ts(
    defaults: dict, *, mcp_tools: list[dict] | None = None,
) -> str:
    """Generate frontend/src/lib/mock-settings.generated.ts from defaults.json.

    Produces:
    - mockSettings: flat array of ResolvedSetting objects
    - buildMockTree(): returns the SettingsNode tree
    - MOCK_MCP_SERVERS: from defaults.json mcp section
    - MOCK_MCP_TOOLS: from mcp-tools.json (Rust-exported tool defs)
    - MOCK_MCP_POLICY: default allow policy
    """
    settings_obj = defaults.get("settings", {})

    # Collect flat settings
    all_settings = _collect_mock_settings(settings_obj, "", "", None)

    # Build mockSettings array
    lines = [
        "// AUTO-GENERATED by scripts/generate_schema.py -- DO NOT EDIT",
        "// Source: config/defaults.json (from guest/config/*.toml)",
        "//",
        "// Regenerate: just run (or just test)",
        "",
        "import type { ResolvedSetting, SettingsNode, McpServerInfo,"
        " McpToolInfo, McpPolicyInfo } from './types';",
        "",
        "// Helper: creates a mock setting with sensible defaults for empty fields.",
        "function ms(overrides: Partial<ResolvedSetting> & {"
        " id: string; category: string; name: string;"
        " setting_type: ResolvedSetting['setting_type'] }): ResolvedSetting {",
        "  return {",
        "    description: '',",
        "    default_value: overrides.setting_type === 'bool' ? false"
        " : overrides.setting_type === 'number' ? 0 : '',",
        "    effective_value: overrides.setting_type === 'bool' ? false"
        " : overrides.setting_type === 'number' ? 0 : '',",
        "    source: 'default',",
        "    modified: null,",
        "    corp_locked: false,",
        "    enabled_by: null,",
        "    enabled: true,",
        "    metadata: { domains: [], choices: [], min: null, max: null, rules: {} },",
        "    ...overrides,",
        "  };",
        "}",
        "",
        "// Helper: wrap a flat ResolvedSetting into a SettingsLeaf node.",
        "function leaf(s: ResolvedSetting): SettingsNode {",
        "  return { kind: 'leaf', ...s };",
        "}",
        "",
    ]

    # Emit mockSettings array
    lines.append("export let mockSettings: ResolvedSetting[] = [")
    for s in all_settings:
        parts = [
            f"    id: {_ts_value(s['id'])}",
            f"category: {_ts_value(s['category'])}",
            f"name: {_ts_value(s['name'])}",
            f"setting_type: {_ts_value(s['setting_type'])}",
        ]
        if s["description"]:
            parts.append(f"description: {_ts_value(s['description'])}")
        parts.append(f"default_value: {_ts_value(s['default_value'])}")
        parts.append(f"effective_value: {_ts_value(s['effective_value'])}")
        if s["enabled_by"]:
            parts.append(f"enabled_by: {_ts_value(s['enabled_by'])}")
            parts.append(f"enabled: {_ts_value(s['enabled'])}")
        meta = s["metadata"]
        # Only emit metadata if it has non-default values
        has_custom = (
            meta.get("domains")
            or meta.get("choices")
            or meta.get("min") is not None
            or meta.get("max") is not None
            or meta.get("rules")
            or meta.get("docs_url")
            or meta.get("prefix")
            or meta.get("filetype")
            or meta.get("format")
            or meta.get("widget")
            or meta.get("side_effect")
            or meta.get("step") is not None
        )
        if has_custom:
            parts.append(f"metadata: {_ts_meta(meta)}")
        lines.append(f"  ms({{ {', '.join(parts)} }}),")
    lines.append("];")
    lines.append("")

    # Emit recomputeEnabled
    lines.extend([
        "/** Recompute `enabled` flags based on parent toggle values. */",
        "export function recomputeEnabled() {",
        "  const values = new Map<string, boolean>();",
        "  for (const s of mockSettings) {",
        "    if (typeof s.effective_value === 'boolean') {",
        "      values.set(s.id, s.effective_value as boolean);",
        "    }",
        "  }",
        "  for (const s of mockSettings) {",
        "    if (s.enabled_by) {",
        "      s.enabled = values.get(s.enabled_by) ?? false;",
        "    }",
        "  }",
        "}",
        "",
    ])

    # Emit buildMockTree
    lines.append("export function buildMockTree(): SettingsNode[] {")
    lines.append("  return [")
    for key, val in settings_obj.items():
        if key in ("name", "description", "enabled_by", "collapsed"):
            continue
        if isinstance(val, dict):
            tree_lines = _build_mock_tree_ts(val, key, None, 2)
            lines.extend(tree_lines)
    lines.append("  ];")
    lines.append("}")
    lines.append("")

    # -- MCP mock data --
    mcp_servers = defaults.get("mcp", {})
    tools = mcp_tools or []

    lines.append("// ---------------------------------------------------------------------------")
    lines.append("// MCP mock data (generated from defaults.json + config/mcp-tools.json)")
    lines.append("// ---------------------------------------------------------------------------")
    lines.append("")

    # MOCK_MCP_SERVERS -- only external servers (user-added SSE/stdio).
    # Builtin servers (like capsem) don't appear in the server list -- their
    # tools are shown in the Local Tools section via MOCK_MCP_TOOLS.
    lines.append("export let MOCK_MCP_SERVERS: McpServerInfo[] = [];")
    lines.append("")

    # MOCK_MCP_TOOLS
    lines.append("export let MOCK_MCP_TOOLS: McpToolInfo[] = [")
    for tool in tools:
        ann = tool.get("annotations")
        ann_ts = "null"
        if ann:
            ann_ts = (
                "{ "
                f"title: {_ts_value(ann.get('title'))}, "
                f"read_only_hint: {_ts_value(ann.get('read_only_hint', False))}, "
                f"destructive_hint: {_ts_value(ann.get('destructive_hint', False))}, "
                f"idempotent_hint: {_ts_value(ann.get('idempotent_hint', False))}, "
                f"open_world_hint: {_ts_value(ann.get('open_world_hint', False))}"
                " }"
            )
        lines.append("  {")
        lines.append(f"    namespaced_name: {_ts_value(tool['namespaced_name'])},")
        lines.append(f"    original_name: {_ts_value(tool['original_name'])},")
        desc = tool.get("description") or ""
        # Truncate long descriptions for mock readability
        if len(desc) > 120:
            desc = desc[:117] + "..."
        lines.append(f"    description: {_ts_value(desc)},")
        lines.append(f"    server_name: {_ts_value(tool.get('server_name', 'builtin'))},")
        lines.append(f"    annotations: {ann_ts},")
        lines.append(f"    pin_hash: null,")
        lines.append(f"    approved: true,")
        lines.append(f"    pin_changed: false,")
        lines.append("  },")
    lines.append("];")
    lines.append("")

    # MOCK_MCP_POLICY
    lines.append("export const MOCK_MCP_POLICY: McpPolicyInfo = {")
    lines.append("  global_policy: 'allow',")
    lines.append("  default_tool_permission: 'allow',")
    lines.append("  blocked_servers: [],")
    lines.append("  tool_permissions: {},")
    lines.append("};")
    lines.append("")

    return "\n".join(lines)
