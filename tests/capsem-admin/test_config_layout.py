"""Config source-layout contract for corp/settings authority."""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CONFIG_ROOT = PROJECT_ROOT / "config"


def test_config_top_level_contract_is_boring_and_explicit() -> None:
    dirs = {path.name for path in CONFIG_ROOT.iterdir() if path.is_dir()}
    assert dirs == {"settings", "corp", "docker", "data"}

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

