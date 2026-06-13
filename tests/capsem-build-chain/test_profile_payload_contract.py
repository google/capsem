"""Profile payload contracts that must hold before image materialization."""

from __future__ import annotations

import tomllib
import json
from pathlib import Path

import blake3


PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROFILES_DIR = PROJECT_ROOT / "config" / "profiles"


def _profile_payload(profile_dir: Path) -> tuple[dict, Path, Path]:
    profile_path = profile_dir / "profile.toml"
    profile = tomllib.loads(profile_path.read_text())
    build_path = PROJECT_ROOT / "config" / profile["files"]["build"]["path"]
    requirements_path = PROJECT_ROOT / "config" / profile["files"]["python_requirements"]["path"]
    return profile, build_path, requirements_path


def test_profiles_ship_ollama_without_cuda_payload_bloat() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile, build_path, requirements_path = _profile_payload(profile_dir)
        build_script = build_path.read_text()
        requirements = {
            line.strip()
            for line in requirements_path.read_text().splitlines()
            if line.strip() and not line.startswith("#")
        }

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
        if 'install_from_url "https://claude.ai/install.sh" "claude"' not in build_script:
            failures.append(f"{profile_id}: build script does not install Claude")
        if 'install -m 555 "/root/.local/bin/$name" "/usr/local/bin/$name"' not in build_script:
            failures.append(
                f"{profile_id}: build script does not promote CLI binaries to /usr/local/bin"
            )

    assert not failures, "invalid Claude permissions bootstrap contract:\n" + "\n".join(failures)


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
        "root/.claude.json",
        "root/.claude/settings.json",
        "root/.claude/settings.local.json",
        "root/.codex/config.toml",
        "root/.mcp.json",
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
