"""Settings/corp ontology contract for the checked-in config defaults.

Runtime policy is the built-in defaults, the user's settings.toml and the
corp config. The shipped settings defaults are UI preferences only; the
shipped corp config owns reporting constraints and plugins.
"""

from __future__ import annotations

import tomllib
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CONFIG_ROOT = PROJECT_ROOT / "config"
SETTINGS_PATH = CONFIG_ROOT / "settings" / "settings.toml"
CORP_PATH = CONFIG_ROOT / "corp" / "corp.toml"


def _toml(path: Path) -> dict[str, Any]:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def test_shipped_settings_are_ui_preferences_not_runtime_policy() -> None:
    settings = _toml(SETTINGS_PATH)

    assert set(settings) == {"app", "appearance"}
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


def test_shipped_corp_owns_reporting_constraints_and_plugins_not_assets_or_ui() -> None:
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
