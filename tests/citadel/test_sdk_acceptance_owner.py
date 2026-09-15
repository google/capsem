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


def _problems(files: dict[str, str], ignored: list[str]) -> list[str]:
    problems = [path for path in (BRAAVOS, *DRIVERS.values()) if path not in files]
    suite = files.get(BRAAVOS, "")
    if not all(language in suite for language in DRIVERS):
        problems.append("three-language parameterization")
    if not all(token in suite for token in ("GatewayInstance", "SDK_GATEWAY_TOKEN", "BRAAVOS_SDK_ACCEPTANCE_OK")):
        problems.append("authenticated gateway fixture")
    required = ("incorrect-token", "profiles", "panics", "triage", "snapshots", "copy")
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
