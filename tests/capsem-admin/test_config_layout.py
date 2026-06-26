"""Config source-layout contract for profile/corp/settings authority."""

from __future__ import annotations

import re
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
CONFIG_ROOT = PROJECT_ROOT / "config"


def test_config_top_level_contract_is_boring_and_explicit() -> None:
    dirs = {path.name for path in CONFIG_ROOT.iterdir() if path.is_dir()}
    assert dirs == {"settings", "corp", "profiles", "docker", "data"}

    forbidden_dirs = {
        "admin",
        "default",
        "defaults",
        "guest",
        "preset",
        "presets",
        "registry",
        "schemas",
        "templates",
        "skills",
    }
    offenders = [
        str(path.relative_to(PROJECT_ROOT))
        for path in CONFIG_ROOT.rglob("*")
        if path.is_dir() and path.name in forbidden_dirs
    ]
    assert offenders == []


def test_config_tree_contains_no_host_metadata_files() -> None:
    offenders = [
        str(path.relative_to(PROJECT_ROOT))
        for path in CONFIG_ROOT.rglob("*")
        if path.name in {".DS_Store", "Thumbs.db"}
    ]
    assert offenders == []


def test_settings_source_is_ui_preferences_only() -> None:
    files = {path.name for path in (CONFIG_ROOT / "settings").iterdir() if path.is_file()}
    assert "settings.toml" in files
    assert files <= {
        "settings.toml",
        "schema.generated.json",
        "ui-metadata.toml",
        "ui-metadata.generated.json",
    }


def test_profiles_own_required_payload_files_without_generated_pins() -> None:
    profile_dirs = sorted(path for path in (CONFIG_ROOT / "profiles").iterdir() if path.is_dir())
    assert profile_dirs, "expected checked-in profiles"

    required_files = {
        "profile.toml",
        "enforcement.toml",
        "detection.yaml",
        "mcp.json",
        "apt-packages.txt",
        "python-requirements.txt",
        "npm-packages.txt",
        "build.sh",
        "tips.txt",
        "root.manifest.json",
    }
    forbidden_pin = re.compile(r"(?m)^\s*(hash|size)\s=")
    failures: list[str] = []
    for profile_dir in profile_dirs:
        present = {path.name for path in profile_dir.iterdir() if path.is_file()}
        missing = required_files - present
        if missing:
            failures.append(f"{profile_dir.relative_to(PROJECT_ROOT)} missing {sorted(missing)}")
        profile_toml = profile_dir / "profile.toml"
        if forbidden_pin.search(profile_toml.read_text()):
            failures.append(f"{profile_toml.relative_to(PROJECT_ROOT)} contains generated pins")

    assert failures == []
