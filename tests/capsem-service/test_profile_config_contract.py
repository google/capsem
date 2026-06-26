"""Profile/corp/settings ontology contract used by service-facing config tests."""

from __future__ import annotations

import json
import re
import tomllib
from pathlib import Path
from typing import Any


PROJECT_ROOT = Path(__file__).resolve().parents[2]
CONFIG_ROOT = PROJECT_ROOT / "config"
SETTINGS_PATH = CONFIG_ROOT / "settings" / "settings.toml"
CORP_PATH = CONFIG_ROOT / "corp" / "corp.toml"
PROFILES_ROOT = CONFIG_ROOT / "profiles"


def _toml(path: Path) -> dict[str, Any]:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def _profile_dirs() -> list[Path]:
    profile_dirs = sorted(path for path in PROFILES_ROOT.iterdir() if path.is_dir())
    assert profile_dirs, "expected checked-in profile directories"
    return profile_dirs


def test_settings_are_ui_preferences_not_runtime_profile_policy() -> None:
    settings = _toml(SETTINGS_PATH)

    assert set(settings) == {"app", "appearance"}
    assert "profiles" not in settings
    assert "corp" not in settings
    assert "rules" not in settings
    assert "plugins" not in settings
    assert "mcp" not in settings
    assert "assets" not in settings

    serialized = SETTINGS_PATH.read_text(encoding="utf-8")
    forbidden = [
        "enforcement",
        "detection",
        "manifest",
        "rootfs",
        "initrd",
        "vmlinuz",
        "credential_broker",
        "log_sanitizer",
    ]
    offenders = [needle for needle in forbidden if needle in serialized]
    assert offenders == []


def test_corp_owns_reporting_constraints_and_plugins_not_profile_assets_or_ui() -> None:
    corp = _toml(CORP_PATH)

    assert corp["refresh_policy"] == "24h"
    assert set(corp["corp_rule_files"]) >= {
        "enforcement",
        "sigma",
        "sigma_output_endpoint",
        "open_telemetry",
        "remote_enforcement",
    }
    assert set(corp["plugins"]) >= {"credential_broker", "log_sanitizer"}

    forbidden_roots = {
        "app",
        "appearance",
        "availability",
        "assets",
        "mcp",
        "rule_files",
        "files",
        "vm",
    }
    assert forbidden_roots.isdisjoint(corp)


def test_profiles_own_assets_rules_mcp_packages_plugins_and_visible_identity() -> None:
    failures: list[str] = []
    required_file_refs = {
        "enforcement": "enforcement.toml",
        "detection": "detection.yaml",
        "mcp": "mcp.json",
        "apt_packages": "apt-packages.txt",
        "python_requirements": "python-requirements.txt",
        "npm_packages": "npm-packages.txt",
        "build": "build.sh",
        "tips": "tips.txt",
        "root_manifest": "root.manifest.json",
    }
    forbidden_pin = re.compile(r"(?m)^\s*(hash|size)\s=")

    for profile_dir in _profile_dirs():
        profile_id = profile_dir.name
        profile_path = profile_dir / "profile.toml"
        profile = _toml(profile_path)

        if profile.get("id") != profile_id:
            failures.append(f"{profile_id}: profile id does not match directory name")
        for key in ["name", "description", "icon_svg", "revision", "refresh_policy"]:
            if not profile.get(key):
                failures.append(f"{profile_id}: missing visible profile field {key}")
        if forbidden_pin.search(profile_path.read_text(encoding="utf-8")):
            failures.append(f"{profile_id}: source profile.toml contains generated hash/size pins")

        if set(profile.get("availability", {})) != {"web", "shell", "mobile"}:
            failures.append(f"{profile_id}: availability must declare web/shell/mobile")
        if set(profile.get("plugins", {})) < {"credential_broker", "log_sanitizer"}:
            failures.append(f"{profile_id}: required plugins are not profile-owned")
        if "server_enabled" not in profile.get("mcp", {}):
            failures.append(f"{profile_id}: MCP server enablement is not profile-owned")

        for arch, assets in profile.get("assets", {}).get("arch", {}).items():
            for asset_name in ["kernel", "initrd", "rootfs"]:
                asset = assets.get(asset_name, {})
                if asset.get("hash") or asset.get("size"):
                    failures.append(
                        f"{profile_id}/{arch}/{asset_name}: source asset has generated pins"
                    )
                if not asset.get("url") or not asset.get("name"):
                    failures.append(f"{profile_id}/{arch}/{asset_name}: missing name/url")

        files = profile.get("files", {})
        for field, filename in required_file_refs.items():
            path = files.get(field, {}).get("path")
            expected = f"profiles/{profile_id}/{filename}"
            if path != expected:
                failures.append(f"{profile_id}: files.{field}.path={path!r}, expected {expected!r}")
                continue
            if not (CONFIG_ROOT / path).is_file():
                failures.append(f"{profile_id}: missing referenced file {path}")

        rule_files = profile.get("rule_files", {})
        if rule_files.get("enforcement") != f"profiles/{profile_id}/enforcement.toml":
            failures.append(f"{profile_id}: enforcement rule file is not profile-owned")
        if rule_files.get("sigma") != f"profiles/{profile_id}/detection.yaml":
            failures.append(f"{profile_id}: detection rule file is not profile-owned")

        mcp = json.loads((profile_dir / "mcp.json").read_text(encoding="utf-8"))
        if "mcpServers" not in mcp:
            failures.append(f"{profile_id}: mcp.json does not expose mcpServers")

    assert failures == []


def test_generated_target_profiles_are_the_only_checked_materialized_profiles() -> None:
    target_profiles = PROJECT_ROOT / "target" / "config" / "profiles"
    assert target_profiles.is_dir(), "runtime profiles must be materialized before service tests"

    source_ids = {path.name for path in _profile_dirs()}
    generated_ids = {path.name for path in target_profiles.iterdir() if path.is_dir()}
    assert source_ids <= generated_ids

    for profile_id in sorted(source_ids):
        generated = target_profiles / profile_id / "profile.toml"
        assert generated.is_file(), f"missing materialized profile {generated}"
        generated_text = generated.read_text(encoding="utf-8")
        assert "hash =" in generated_text
        assert "size =" in generated_text
