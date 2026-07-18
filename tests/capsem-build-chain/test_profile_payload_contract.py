"""Profile payload contracts that must hold before image materialization."""

from __future__ import annotations

import tomllib
import json
from pathlib import Path

import blake3


PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROFILES_DIR = PROJECT_ROOT / "config" / "profiles"
MATERIALIZED_PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"


def _profile_payload(profile_dir: Path) -> tuple[dict, Path, Path]:
    profile_path = profile_dir / "profile.toml"
    profile = tomllib.loads(profile_path.read_text())
    build_path = PROJECT_ROOT / "config" / profile["files"]["build"]["path"]
    requirements_path = PROJECT_ROOT / "config" / profile["files"]["python_requirements"]["path"]
    return profile, build_path, requirements_path


def _package_lines(path: Path) -> set[str]:
    return {
        line.strip()
        for line in path.read_text().splitlines()
        if line.strip() and not line.startswith("#")
    }


def test_profiles_package_ai_cli_sandbox_prerequisites() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile = tomllib.loads((profile_dir / "profile.toml").read_text())
        profile_id = profile["id"]
        apt_path = PROJECT_ROOT / "config" / profile["files"]["apt_packages"]["path"]
        apt_packages = _package_lines(apt_path)
        if "bubblewrap" not in apt_packages:
            failures.append(f"{profile_id}: apt packages do not include bubblewrap")

    assert not failures, "invalid AI CLI sandbox prerequisite contract:\n" + "\n".join(failures)


def test_profiles_ship_ollama_without_cuda_payload_bloat() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile, build_path, requirements_path = _profile_payload(profile_dir)
        build_script = build_path.read_text()
        requirements = _package_lines(requirements_path)

        profile_id = profile["id"]
        if "https://ollama.com/install.sh" not in build_script:
            failures.append(f"{profile_id}: build script does not install Ollama")
        if "rm -rf /usr/local/lib/ollama/cuda_*" not in build_script:
            failures.append(f"{profile_id}: build script does not prune Ollama CUDA libraries")
        if "ollama" not in requirements:
            failures.append(f"{profile_id}: python requirements do not include the Ollama SDK")

    assert not failures, "invalid profile payload contract:\n" + "\n".join(failures)


def _root_manifest_entries(profile_dir: Path) -> dict[str, dict]:
    manifest = json.loads((profile_dir / "root.manifest.json").read_text())
    assert manifest["format"] == "capsem.profile-root.v1"
    return {entry["path"]: entry for entry in manifest["files"]}


def test_profiles_package_claude_mcp_approval_when_capsem_mcp_is_declared() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile, _, _ = _profile_payload(profile_dir)
        profile_id = profile["id"]
        root_dir = profile_dir / "root"
        mcp = json.loads((root_dir / "root/.mcp.json").read_text())
        if "capsem" not in mcp.get("mcpServers", {}):
            continue

        approval_rel = "root/.claude/settings.local.json"
        approval_path = root_dir / approval_rel
        if not approval_path.is_file():
            failures.append(f"{profile_id}: missing {approval_rel}")
            continue

        approval = json.loads(approval_path.read_text())
        if "capsem" not in approval.get("enabledMcpjsonServers", []):
            failures.append(f"{profile_id}: {approval_rel} does not approve capsem MCP")

        entries = _root_manifest_entries(profile_dir)
        manifest_entry = entries.get(approval_rel)
        if manifest_entry is None:
            failures.append(f"{profile_id}: root manifest does not pin {approval_rel}")
            continue
        payload = approval_path.read_bytes()
        expected_hash = "blake3:" + blake3.blake3(payload).hexdigest()
        if manifest_entry.get("hash") != expected_hash:
            failures.append(f"{profile_id}: {approval_rel} manifest hash is stale")
        if manifest_entry.get("size") != len(payload):
            failures.append(f"{profile_id}: {approval_rel} manifest size is stale")

    assert not failures, "invalid Claude MCP bootstrap contract:\n" + "\n".join(failures)


def test_profiles_package_claude_bypass_permissions_bootstrap() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile, build_path, _ = _profile_payload(profile_dir)
        profile_id = profile["id"]
        build_script = build_path.read_text()
        settings_path = profile_dir / "root/root/.claude/settings.json"
        if not settings_path.is_file():
            failures.append(f"{profile_id}: missing root/.claude/settings.json")
            continue
        settings = json.loads(settings_path.read_text())
        default_mode = settings.get("permissions", {}).get("defaultMode")
        if default_mode != "bypassPermissions":
            failures.append(
                f"{profile_id}: Claude defaultMode is {default_mode!r}, expected bypassPermissions"
            )
        state_path = profile_dir / "root/root/.claude.json"
        if not state_path.is_file():
            failures.append(f"{profile_id}: missing root/.claude.json")
        else:
            state = json.loads(state_path.read_text())
            if state.get("installMethod") != "native":
                failures.append(
                    f"{profile_id}: Claude installMethod is {state.get('installMethod')!r}, expected native"
                )
            if state.get("autoUpdates") is not False:
                failures.append(f"{profile_id}: Claude autoUpdates must be false")
            if state.get("autoUpdatesProtectedForNative") is not True:
                failures.append(
                    f"{profile_id}: Claude autoUpdatesProtectedForNative must be true"
                )
        if 'install_from_url "https://claude.ai/install.sh" "claude"' not in build_script:
            failures.append(f"{profile_id}: build script does not install Claude")
        if 'install -m 555 "/root/.local/bin/$name" "/usr/local/bin/$name"' not in build_script:
            failures.append(
                f"{profile_id}: build script does not promote CLI binaries to /usr/local/bin"
            )
        shell_paths = {
            "root/.bashrc": profile_dir / "root/root/.bashrc",
            "root/.profile": profile_dir / "root/root/.profile",
        }
        for rel, shell_path in shell_paths.items():
            if not shell_path.is_file():
                failures.append(f"{profile_id}: missing {rel}")
                continue
            shell = shell_path.read_text()
            expected_path_prefix = 'PATH="/opt/ai-clis/bin:/usr/local/bin:/root/.local/bin:'
            if expected_path_prefix not in shell:
                failures.append(
                    f"{profile_id}: {rel} must keep /opt/ai-clis/bin on PATH "
                    "before durable /usr/local/bin and /root/.local/bin"
                )
            if "export PATH" not in shell:
                failures.append(f"{profile_id}: {rel} does not export PATH")

        claude_shim_rel = "root/.local/bin/claude"
        claude_shim_path = profile_dir / "root" / claude_shim_rel
        if claude_shim_path.exists():
            failures.append(
                f"{profile_id}: {claude_shim_rel} must not be baked into the profile root; "
                "Claude should resolve to the durable /usr/local/bin binary"
            )

    assert not failures, "invalid Claude permissions bootstrap contract:\n" + "\n".join(failures)


FORBIDDEN_SHIPPED_PROVIDER_FRAGMENTS = (
    "127.0.0.1:11434",
    "localhost:11434",
    "CAPSEM_MOCK_SERVER",
    '"provider": "ollama"',
    '"baseUrl": "http://127.0.0.1:11434"',
)


def test_profiles_package_scriptable_agent_bootstrap_without_local_provider_leakage() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile_id = profile_dir.name
        root_dir = profile_dir / "root"

        codex_path = root_dir / "root/.codex/config.toml"
        codex = tomllib.loads(codex_path.read_text())
        if codex.get("model_provider") in {"local_ollama", "ollama"}:
            failures.append(f"{profile_id}: Codex default must not force Ollama")
        providers = codex.get("model_providers", {})
        if "local_ollama" in providers or "ollama" in providers:
            failures.append(f"{profile_id}: Codex config must not hide an Ollama provider")
        if codex.get("check_for_update_on_startup") is not False:
            failures.append(f"{profile_id}: Codex startup update checks must be disabled")
        analytics = codex.get("analytics", {})
        if analytics.get("enabled") is not False:
            failures.append(f"{profile_id}: Codex analytics must be disabled")

        agy_config_path = root_dir / "root/.gemini/config/config.json"
        if not agy_config_path.is_file():
            failures.append(f"{profile_id}: missing root/.gemini/config/config.json")
        else:
            agy_config = json.loads(agy_config_path.read_text())
            ai = agy_config.get("ai", {})
            if ai:
                failures.append(
                    f"{profile_id}: AGY config must not force a model provider"
                )
            if "auth" in ai or "token" in json.dumps(ai).lower():
                failures.append(f"{profile_id}: AGY local model config bakes auth material")

        agy_cli_settings_path = root_dir / "root/.gemini/antigravity-cli/settings.json"
        if not agy_cli_settings_path.is_file():
            failures.append(f"{profile_id}: missing root/.gemini/antigravity-cli/settings.json")
        else:
            agy_cli_settings = json.loads(agy_cli_settings_path.read_text())
            if "model" in agy_cli_settings:
                failures.append(
                    f"{profile_id}: AGY CLI settings must not pin model; "
                    "agy 1.0.8 rejects the nested model setting"
                )
            if "toolPermission" in agy_cli_settings:
                failures.append(f"{profile_id}: AGY CLI settings include invalid toolPermission")
            if "/root" not in agy_cli_settings.get("trustedWorkspaces", []):
                failures.append(f"{profile_id}: AGY CLI settings do not trust /root")
            if agy_cli_settings.get("telemetry", {}).get("enabled") is not False:
                failures.append(f"{profile_id}: AGY CLI telemetry is not disabled")
            if agy_cli_settings.get("autoUpdate", {}).get("enabled") is not False:
                failures.append(f"{profile_id}: AGY CLI autoUpdate is not disabled")
            if "auth" in agy_cli_settings or "token" in json.dumps(agy_cli_settings).lower():
                failures.append(f"{profile_id}: AGY CLI settings bake auth material")

    assert not failures, "invalid scriptable agent bootstrap contract:\n" + "\n".join(failures)


def test_shipped_profile_roots_do_not_contain_test_or_local_provider_overrides() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile_id = profile_dir.name
        root_dir = profile_dir / "root"
        for path in sorted(root_dir.rglob("*")):
            if not path.is_file():
                continue
            text = path.read_text(errors="ignore")
            for fragment in FORBIDDEN_SHIPPED_PROVIDER_FRAGMENTS:
                if fragment in text:
                    rel = path.relative_to(root_dir).as_posix()
                    failures.append(f"{profile_id}: {rel} contains {fragment!r}")

    assert not failures, "profile root provider leakage:\n" + "\n".join(failures)


def test_profile_root_manifests_pin_exactly_the_shipped_root_payload() -> None:
    failures: list[str] = []
    forbidden_path_fragments = (
        "oauth",
        "token",
        "conversation",
        "history",
        "cache",
        ".log",
    )
    required_payloads = {
        "root/.antigravity/settings.json",
        "root/.bashrc",
        "root/.claude.json",
        "root/.claude/settings.json",
        "root/.claude/settings.local.json",
        "root/.codex/config.toml",
        "root/.gemini/antigravity-cli/settings.json",
        "root/.gemini/config/config.json",
        "root/.mcp.json",
        "root/.profile",
    }

    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile_id = profile_dir.name
        root_dir = profile_dir / "root"
        manifest_entries = _root_manifest_entries(profile_dir)
        actual_paths = {
            path.relative_to(root_dir).as_posix()
            for path in root_dir.rglob("*")
            if path.is_file()
        }
        manifest_paths = set(manifest_entries)

        missing = sorted(actual_paths - manifest_paths)
        if missing:
            failures.append(f"{profile_id}: unpinned root payload files: {missing}")
        stale = sorted(manifest_paths - actual_paths)
        if stale:
            failures.append(f"{profile_id}: manifest lists missing root payload files: {stale}")

        for required in sorted(required_payloads):
            if required not in actual_paths:
                failures.append(f"{profile_id}: missing non-secret bootstrap payload {required}")

        for rel in sorted(actual_paths):
            lowered = rel.lower()
            if any(fragment in lowered for fragment in forbidden_path_fragments):
                failures.append(f"{profile_id}: forbidden secret/cache/log payload path {rel}")
                continue
            payload = (root_dir / rel).read_bytes()
            entry = manifest_entries.get(rel)
            if entry is None:
                continue
            expected_hash = "blake3:" + blake3.blake3(payload).hexdigest()
            if entry.get("hash") != expected_hash:
                failures.append(f"{profile_id}: {rel} manifest hash is stale")
            if entry.get("size") != len(payload):
                failures.append(f"{profile_id}: {rel} manifest size is stale")

    assert not failures, "invalid profile root payload contract:\n" + "\n".join(failures)


def test_materialized_profile_root_payload_matches_source_profile_root() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile_id = profile_dir.name
        materialized_dir = MATERIALIZED_PROFILES_DIR / profile_id
        if not materialized_dir.is_dir():
            failures.append(f"{profile_id}: missing materialized profile directory")
            continue

        source_root = profile_dir / "root"
        materialized_root = materialized_dir / "root"
        source_paths = {
            path.relative_to(source_root).as_posix()
            for path in source_root.rglob("*")
            if path.is_file()
        }
        materialized_paths = {
            path.relative_to(materialized_root).as_posix()
            for path in materialized_root.rglob("*")
            if path.is_file()
        }
        if source_paths != materialized_paths:
            missing = sorted(source_paths - materialized_paths)
            extra = sorted(materialized_paths - source_paths)
            failures.append(
                f"{profile_id}: materialized root payload drift missing={missing} extra={extra}"
            )
            continue
        for rel in sorted(source_paths):
            source_bytes = (source_root / rel).read_bytes()
            materialized_bytes = (materialized_root / rel).read_bytes()
            if materialized_bytes != source_bytes:
                failures.append(f"{profile_id}: materialized root payload differs for {rel}")

    assert not failures, "materialized profile root drift:\n" + "\n".join(failures)


def test_profiles_package_agent_bootstrap_without_baking_credentials() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile, build_path, _ = _profile_payload(profile_dir)
        profile_id = profile["id"]
        root_dir = profile_dir / "root"

        agy_settings = json.loads((root_dir / "root/.antigravity/settings.json").read_text())
        if "/root" not in agy_settings.get("trustedWorkspaces", []):
            failures.append(f"{profile_id}: AGY does not trust /root workspace")
        if "auth" in agy_settings or "token" in json.dumps(agy_settings).lower():
            failures.append(f"{profile_id}: AGY settings bake auth material")

        build_script = build_path.read_text()
        required_cleanup_paths = [
            "/root/.antigravity/*oauth*",
            "/root/.antigravity/*token*",
            "/root/.claude/cache",
            "/root/.claude/history",
            "/root/.codex/cache",
            "/root/.codex/history",
            "/root/.gemini/cache",
            "/root/.gemini/history",
            "/root/.gemini/logs",
            "/root/.gemini/tmp",
        ]
        if "cleanup_agent_runtime_state" not in build_script:
            failures.append(f"{profile_id}: build script does not define agent runtime cleanup")
        for cleanup_path in required_cleanup_paths:
            if cleanup_path not in build_script:
                failures.append(f"{profile_id}: build script does not clean {cleanup_path}")
        if "agy-real" not in build_script:
            failures.append(f"{profile_id}: AGY wrapper does not preserve vendor binary as agy-real")
        if "--dangerously-skip-permissions" not in build_script:
            failures.append(f"{profile_id}: AGY wrapper does not enable Capsem sandbox mode")
        if "gemini-real" not in build_script:
            failures.append(f"{profile_id}: Gemini wrapper does not expose vendor entrypoint as gemini-real")
        if "gemini_target=\"$(readlink -f \"$gemini_path\")\"" not in build_script:
            failures.append(f"{profile_id}: Gemini wrapper does not resolve the real npm entrypoint")
        if 'ln -sfn "$gemini_target" "$gemini_dir/gemini-real"' not in build_script:
            failures.append(f"{profile_id}: Gemini wrapper does not preserve vendor entrypoint by symlink")
        if 'install -m 555 "$gemini_path" "$gemini_dir/gemini-real"' in build_script:
            failures.append(f"{profile_id}: Gemini wrapper copies the JS entrypoint and breaks relative imports")
        if "cleanup_gemini_runtime_state" not in build_script:
            failures.append(f"{profile_id}: Gemini wrapper does not clean CLI runtime residue")

        codex = tomllib.loads((root_dir / "root/.codex/config.toml").read_text())
        command = codex.get("mcp_servers", {}).get("capsem", {}).get("command")
        if command != "/run/capsem-mcp-server":
            failures.append(f"{profile_id}: Codex capsem MCP command is {command!r}")

    assert not failures, "invalid agent bootstrap contract:\n" + "\n".join(failures)


def test_profiles_pin_verified_agy_release_for_both_linux_architectures() -> None:
    failures: list[str] = []
    required_fragments = (
        'AGY_VERSION="1.1.3"',
        "agy_cli_linux_arm64.tar.gz",
        "agy_cli_linux_x64.tar.gz",
        "453f9c5530877ab6369e2536e576cfab2bbbcb45923a9bc776678142538e419d",
        "7a7239a69b65d3cf3af7e75f27b2ff4e9cce696a7b9a9e5c37c695f1c74eec34",
        "github.com/google-antigravity/antigravity-cli/releases/download/$AGY_VERSION/$asset",
        "sha256sum -c -",
        'install -m 555 "$tmp/antigravity" /usr/local/bin/agy',
    )
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile, build_path, _ = _profile_payload(profile_dir)
        build_script = build_path.read_text()
        for fragment in required_fragments:
            if fragment not in build_script:
                failures.append(f"{profile['id']}: AGY installer missing {fragment!r}")
        if "antigravity.google/cli/install.sh" in build_script:
            failures.append(f"{profile['id']}: AGY still uses mutable broken installer URL")
        if "releases/latest" in build_script:
            failures.append(f"{profile['id']}: AGY release is not version pinned")

    assert not failures, "invalid pinned AGY installer contract:\n" + "\n".join(failures)


def test_profiles_allow_only_capsem_mock_server_fixture_over_local_network_guard() -> None:
    failures: list[str] = []
    expected_match = (
        '(http.host == "127.0.0.1" || http.host == "localhost") '
        '&& ip.value == "127.0.0.1" '
        '&& tcp.port == "3713"'
    )
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile, _, _ = _profile_payload(profile_dir)
        profile_id = profile["id"]
        enforcement_path = PROJECT_ROOT / "config" / profile["files"]["enforcement"]["path"]
        rules = tomllib.loads(enforcement_path.read_text())
        mock_rule = rules.get("profiles", {}).get("rules", {}).get("capsem_mock_server")
        if mock_rule is None:
            failures.append(f"{profile_id}: missing profiles.rules.capsem_mock_server")
            continue
        if mock_rule.get("name") != "capsem_mock_server":
            failures.append(f"{profile_id}: mock-server rule name is wrong")
        if mock_rule.get("action") != "allow":
            failures.append(f"{profile_id}: mock-server rule must allow")
        if mock_rule.get("priority") != 10:
            failures.append(f"{profile_id}: mock-server rule priority must be 10")
        if mock_rule.get("match") != expected_match:
            failures.append(f"{profile_id}: mock-server rule match is too broad or stale")
        if "reason" not in mock_rule:
            failures.append(f"{profile_id}: mock-server rule needs a reason")

    assert not failures, "invalid mock-server local-network contract:\n" + "\n".join(failures)
