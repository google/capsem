"""Citadel guard: Rust architecture guidance follows the executable workspace.

The crate split only reduces agent context when the routing documents name the
new owners. Coverage thresholds also stay config-owned so a prose copy cannot
silently disagree with the gate.
"""

from __future__ import annotations

import re
from collections.abc import Mapping
from pathlib import Path

import pytest
import tomllib

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKSPACE_MANIFEST = PROJECT_ROOT / "Cargo.toml"

CRATE_MAP_DOCUMENTS = (
    "AGENTS.md",
    "skills/dev-capsem/SKILL.md",
    "skills/site-architecture/references/crate-and-privilege-model.md",
    "skills/dev-testing/references/test-matrix.md",
)
TESTING_GUIDANCE = (
    "skills/dev-testing/SKILL.md",
    "skills/dev-testing/references/test-matrix.md",
)

CRATE_NAME = re.compile(r"(?<![a-z0-9-])(capsem(?:-[a-z0-9]+)*)(?![a-z0-9-])")
STALE_CORE_CLAIM = re.compile(
    r"(?:capsem-core[^.\n]*(?:all shared logic|all business logic)|"
    r"(?:all shared logic|all business logic)[^.\n]*capsem-core)",
    re.IGNORECASE,
)
COPIED_RUST_COVERAGE_FLOOR = re.compile(
    r"(?:Rust workspace\s*\|\s*\d+%|"
    r"--fail-under-lines(?:=|\s+)\d+|"
    r"floor:\s*\d+%\s+line coverage)",
    re.IGNORECASE,
)

ARCHITECTURE_RATIONALE = """\
The Rust crate map is an agent routing contract, not a historical overview.
Every Cargo workspace member must be named in each canonical map, and no map
may restore the obsolete claim that capsem-core owns every shared concern.
Route reusable code to the lowest-dependency crate that owns its domain.
"""

COVERAGE_RATIONALE = """\
The Rust coverage floor is owned by config/gate.toml. Copying its numeric value
into testing guidance creates a second authority that becomes stale when the
ratchet moves; refer to rust_coverage_floors instead.
"""


def _workspace_crates(manifest: str) -> set[str]:
    members = tomllib.loads(manifest)["workspace"]["members"]
    return {Path(member).name for member in members if member.startswith("crates/")}


def _documented_crates(source: str) -> set[str]:
    return set(CRATE_NAME.findall(source))


def _missing_crate_docs(
    workspace: set[str], documents: Mapping[str, str]
) -> dict[str, list[str]]:
    return {
        path: sorted(workspace - _documented_crates(source))
        for path, source in documents.items()
        if workspace - _documented_crates(source)
    }


def _stale_core_claims(documents: Mapping[str, str]) -> list[str]:
    return sorted(
        path for path, source in documents.items() if STALE_CORE_CLAIM.search(source)
    )


def _copied_coverage_floors(documents: Mapping[str, str]) -> list[str]:
    return sorted(
        path
        for path, source in documents.items()
        if COPIED_RUST_COVERAGE_FLOOR.search(source)
    )


def _read(paths: tuple[str, ...]) -> dict[str, str]:
    return {path: (PROJECT_ROOT / path).read_text() for path in paths}


def test_canonical_crate_maps_cover_the_workspace() -> None:
    workspace = _workspace_crates(WORKSPACE_MANIFEST.read_text())
    missing = _missing_crate_docs(workspace, _read(CRATE_MAP_DOCUMENTS))
    assert not missing, ARCHITECTURE_RATIONALE + f"\nMissing crate entries: {missing}"


def test_crate_maps_do_not_restore_core_as_a_catch_all() -> None:
    violations = _stale_core_claims(_read(CRATE_MAP_DOCUMENTS))
    assert not violations, (
        ARCHITECTURE_RATIONALE + f"\nStale capsem-core claims: {violations}"
    )


def test_testing_guidance_derives_the_rust_coverage_floor() -> None:
    violations = _copied_coverage_floors(_read(TESTING_GUIDANCE))
    assert not violations, (
        COVERAGE_RATIONALE + f"\nCopied numeric coverage floors: {violations}"
    )


def test_crate_map_guard_detects_a_new_undocumented_workspace_member() -> None:
    workspace = _workspace_crates(WORKSPACE_MANIFEST.read_text()) | {
        "capsem-new-owner"
    }
    complete_map = " ".join(f"`{crate}`" for crate in sorted(workspace))
    documents = dict.fromkeys(CRATE_MAP_DOCUMENTS, complete_map)
    documents[CRATE_MAP_DOCUMENTS[0]] = complete_map.replace(
        "`capsem-new-owner`", ""
    )
    assert _missing_crate_docs(workspace, documents)[CRATE_MAP_DOCUMENTS[0]] == [
        "capsem-new-owner"
    ]


@pytest.mark.parametrize(
    "claim",
    (
        "capsem-core owns all business logic.",
        "capsem-core: all shared logic and VM runtime.",
        "All business logic lives in `capsem-core`.",
    ),
)
def test_core_ownership_guard_detects_stale_spellings(claim: str) -> None:
    assert _stale_core_claims({"routing.md": claim}) == ["routing.md"]


@pytest.mark.parametrize(
    "claim",
    (
        "| Rust workspace | 64% | enforced |",
        "cargo llvm-cov --fail-under-lines=64",
        "Rust coverage (floor: 64% line coverage)",
    ),
)
def test_coverage_guard_detects_copied_numeric_floors(claim: str) -> None:
    assert _copied_coverage_floors({"testing.md": claim}) == ["testing.md"]


def test_coverage_guard_accepts_the_config_owned_reference() -> None:
    source = "Rust coverage is owned by config/gate.toml rust_coverage_floors."
    assert not _copied_coverage_floors({"testing.md": source})
