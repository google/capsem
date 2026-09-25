"""Braavos keeps one authenticated, cross-language SDK acceptance owner."""

from __future__ import annotations

import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
BRAAVOS = "tests/ironbank/test_braavos_sdk.py"
DRIVERS = {
    "python": "sdk/python/tests/gateway_acceptance.py",
    "typescript": "sdk/typescript/tools/gateway-acceptance.mjs",
    "rust": "sdk/rust/examples/gateway_acceptance.rs",
}
RATIONALE = (
    "Braavos must exercise every packaged SDK through ordinary authenticated "
    "gateway HTTP and remain discoverable by the broad Ironbank gate."
)
TUI_FILES = {
    "manifest": "crates/capsem-tui/Cargo.toml",
    "provider": "crates/capsem-tui/src/gateway_provider.rs",
    "actions": "crates/capsem-tui/src/sdk_actions.rs",
    "terminal": "crates/capsem-tui/src/terminal.rs",
}
TUI_RATIONALE = (
    "The shipped TUI is the Rust SDK's continuous product consumer: supported "
    "gateway control must use SDK resources, while direct transport remains limited "
    "to token bootstrap and the WebSocket stream the SDK does not yet expose."
)


def _problems(files: dict[str, str], ignored: list[str]) -> list[str]:
    problems = [path for path in (BRAAVOS, *DRIVERS.values()) if path not in files]
    suite = files.get(BRAAVOS, "")
    if not all(f'"{language}"' in suite for language in DRIVERS):
        problems.append("three-language parameterization")
    if not all(token in suite for token in ("GatewayInstance", "SDK_GATEWAY_TOKEN", "BRAAVOS_SDK_ACCEPTANCE_OK")):
        problems.append("authenticated gateway fixture")
    required = ("incorrect-token", "profiles", "panics", "triage", "files")
    for language, path in DRIVERS.items():
        if not all(token in files.get(path, "") for token in required):
            problems.append(f"{language} behavior")
    if "tests/ironbank" in ignored or BRAAVOS in ignored:
        problems.append("broad gate discovery")
    return problems


def _sources() -> dict[str, str]:
    return {path: (ROOT / path).read_text() for path in (BRAAVOS, *DRIVERS.values())}


def test_braavos_owns_cross_language_authenticated_sdk_acceptance() -> None:
    config = tomllib.loads((ROOT / "config/gate.toml").read_text())
    assert not _problems(_sources(), config["suites"]["pytest"]["broad_ignores"]), RATIONALE


@pytest.mark.parametrize("mutation", ["missing", "language", "auth", "behavior", "ignored"])
def test_braavos_ownership_regressions_are_rejected(mutation: str) -> None:
    files, ignored = _sources(), []
    if mutation == "missing":
        files.pop(DRIVERS["rust"])
    elif mutation == "language":
        files[BRAAVOS] = files[BRAAVOS].replace('"typescript"', '"removed"')
    elif mutation == "auth":
        files[BRAAVOS] = files[BRAAVOS].replace("SDK_GATEWAY_TOKEN", "REMOVED_TOKEN")
    elif mutation == "behavior":
        files[DRIVERS["python"]] = files[DRIVERS["python"]].replace("triage", "removed")
    else:
        ignored.append(BRAAVOS)
    assert _problems(files, ignored), RATIONALE


def _tui_problems(files: dict[str, str]) -> list[str]:
    manifest, provider = files["manifest"], files["provider"]
    actions, terminal = files["actions"], files["terminal"]
    problems = []
    if 'capsem-sdk = { path = "../../sdk/rust" }' not in manifest:
        problems.append("SDK dependency")
    if ".profiles()" not in provider or ".purge(*all)" not in provider:
        problems.append("resource facades")
    if "operations::list_profiles" in provider or 'post(join_url(base_url, &["purge"])' in provider:
        problems.append("raw supported control")
    if not all(token in actions for token in ("capsem_sdk::operations", ".fork(", ".stop()", ".delete()")):
        problems.append("typed actions")
    if "capsem_sdk::models::stream" not in terminal:
        problems.append("typed stream protocol")
    return problems


def test_tui_is_the_rust_sdk_product_consumer() -> None:
    files = {name: (ROOT / path).read_text() for name, path in TUI_FILES.items()}
    assert not _tui_problems(files), TUI_RATIONALE


def test_tui_sdk_consumer_guard_rejects_raw_supported_control() -> None:
    files = {name: (ROOT / path).read_text() for name, path in TUI_FILES.items()}
    files["provider"] = files["provider"].replace(".purge(*all)", ".raw_purge(*all)")
    assert _tui_problems(files), TUI_RATIONALE
