"""Offline downgrade guards supplement the live dependency advisory audit."""

from pathlib import Path

import pytest
import yaml
from packaging.version import Version

ROOT = Path(__file__).resolve().parents[2]
WORKSPACES = ("web/app", "web/docs", "web/marketing", "build_system/release_site")
SECURITY_FLOORS = {
    "astro": "7.2.8",  # GHSA-26w7-cxv4-gfx2
    "sharp": "0.35.4",  # GHSA-rgj7-g3m4-5g8c
    "vitest": "4.1.11",  # GHSA-82fw-gwwq-j7x9
    "@vitest/mocker": "4.1.11",
    "js-yaml": "4.3.2",  # GHSA-2883-xcg3-v3hh; this repo overrides to v4
    "svgo": "4.1.0",  # GHSA-w27v-7q3p-w38r; this repo uses v4
}
SECURITY_COHORT_RATIONALE = (
    "The September 2026 macOS baseline stopped at dependency auditing. "
    "Every web lock must retain the patched cohort, including transitive copies; "
    "a clean cached advisory result must not permit a lockfile downgrade. "
    "The live audit remains authoritative for newly published advisories. "
    "See skills/dev-ci/SKILL.md and the GHSA identifiers beside each floor."
)


def _unsafe(packages: dict) -> list[str]:
    findings = []
    for key in packages:
        name, version = key.rsplit("@", 1)
        floor = SECURITY_FLOORS.get(name)
        if floor and Version(version) < Version(floor):
            findings.append(key)
    return findings


@pytest.mark.parametrize("workspace", WORKSPACES)
def test_all_locked_web_copies_retain_security_fixes(workspace: str) -> None:
    lock = yaml.safe_load((ROOT / workspace / "pnpm-lock.yaml").read_text())
    assert lock["packages"], SECURITY_COHORT_RATIONALE
    assert not (unsafe := _unsafe(lock["packages"])), (
        f"{workspace}: {unsafe}\n{SECURITY_COHORT_RATIONALE}"
    )


@pytest.mark.parametrize("name,floor", SECURITY_FLOORS.items())
def test_guard_rejects_an_older_transitive_copy_beside_a_fixed_one(name, floor) -> None:
    version = Version(floor)
    older = f"{version.major}.{version.minor}.{version.micro - 1}"
    if version.micro == 0:
        older = f"{version.major}.{version.minor - 1}.99"
    assert _unsafe({f"{name}@{floor}": {}, f"{name}@{older}": {}}) == [f"{name}@{older}"]
    assert _unsafe({f"{name}@{floor}": {}}) == []
