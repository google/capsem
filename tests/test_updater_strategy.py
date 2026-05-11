"""Release updater strategy contracts.

T0 disables the desktop self-updater until the release publishes a complete
full-install update path. These tests keep config, frontend affordances, and
Tauri permissions from drifting back to a half-enabled updater.
"""

from __future__ import annotations

from pathlib import Path


REPO_ROOT = Path(__file__).parent.parent


def _read(path: str) -> str:
    return (REPO_ROOT / path).read_text()


def test_tauri_updater_is_disabled_for_package_release():
    tauri_conf = _read("crates/capsem-app/tauri.conf.json")
    cargo_toml = _read("crates/capsem-app/Cargo.toml")
    app_main = _read("crates/capsem-app/src/main.rs")
    capabilities = _read("crates/capsem-app/capabilities/default.json")

    forbidden = [
        "createUpdaterArtifacts",
        '"updater"',
        "latest.json",
        "tauri-plugin-updater",
        "tauri_plugin_updater",
        "check_for_update_with_prompt",
        "check_for_app_update",
        "updater:default",
    ]
    combined = "\n".join([tauri_conf, cargo_toml, app_main, capabilities])
    for needle in forbidden:
        assert needle not in combined


def test_update_settings_and_frontend_affordances_are_hidden():
    files = [
        "src/capsem/builder/config.py",
        "config/defaults.json",
        "config/defaults.toml",
        "frontend/src/lib/mock-settings.generated.ts",
        "frontend/src/lib/mock-settings.ts",
        "frontend/src/lib/api.ts",
        "frontend/src/lib/components/settings/SettingsSection.svelte",
        "frontend/src/lib/components/shell/SettingsPage.svelte",
    ]
    combined = "\n".join(_read(path) for path in files)
    for needle in [
        "app.auto_update",
        "app.check_update",
        "Auto-check for updates",
        "Check for updates",
        "checkForAppUpdate",
        "ActionKind.CheckUpdate",
        "0.1.0-dev",
    ]:
        assert needle not in combined
