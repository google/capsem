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
        profile, _, _ = _profile_payload(profile_dir)
        profile_id = profile["id"]
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

    assert not failures, "invalid Claude permissions bootstrap contract:\n" + "\n".join(failures)
