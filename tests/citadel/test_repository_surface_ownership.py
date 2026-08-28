"""Require one engineering owner for every current and target repository surface."""

from __future__ import annotations

import subprocess
import tomllib
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any, cast

import pytest

ROOT = Path(__file__).resolve().parents[2]
POLICY = Path(__file__).with_name("repository_surface_ownership.toml")

OWNERSHIP_RATIONALE = """\
A directory move is incomplete until lint, tests, CI, coverage, and migration
debt agree on its owner. A source tree without all five can pass locally while
being absent from CI or coverage. The SDK is intentionally a separate sprint:
its reserved root must remain empty until that sprint installs its own gates.
"""

EXPECTED_TARGETS = frozenset(
    {
        "<root-files>",
        ".agents/",
        ".cargo/",
        ".claude/",
        ".codex/",
        ".config/",
        ".cursor/",
        ".gemini/",
        ".github/",
        "build_system/builder/",
        "build_system/docker/",
        "build_system/packaging/",
        "build_system/release_site/",
        "build_system/scripts/",
        "build_system/tests/",
        "benchmarks/collectors/",
        "benchmarks/baselines/",
        "config/",
        "crates/",
        "guest/",
        "sdk/",
        "skills/",
        "tests/",
        "web/app/",
        "web/docs/",
        "web/marketing/",
        "web/graphics/",
        "target/",
    }
)
KNOWN_CI_JOBS = frozenset(
    {
        "fast-gate",
        "test-linux",
        "test",
        "test-install",
        "docs-build",
        "site-build",
        "release-site-build",
        "pr-gate",
    }
)
REQUIRED_FIELDS = ("id", "state", "current", "targets", "lint", "tests", "ci", "coverage", "debt", "reason")


def _known_lint_owners() -> frozenset[str]:
    gate_policy = tomllib.loads((ROOT / "config/gate.toml").read_text(encoding="utf-8"))
    return frozenset(
        owner
        for surface in gate_policy["lint_surfaces"]
        for field in ("enforced_by", "checked_by")
        for owner in surface.get(field, [])
    )


def _matches(path: str, declaration: str) -> bool:
    return path.startswith(declaration) if declaration.endswith("/") else path == declaration


def _problems(policy: Mapping[str, Any], tracked: Sequence[str]) -> list[str]:
    rows = policy.get("surface", [])
    if not isinstance(rows, list) or not rows:
        return ["no [[surface]] ownership declarations"]

    problems: list[str] = []
    targets: list[str] = []
    ids: list[str] = []
    known_lint_owners = _known_lint_owners()
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            problems.append(f"surface {index} is not a table")
            continue
        typed_row = cast(dict[str, Any], row)
        missing = [field for field in REQUIRED_FIELDS if field not in typed_row]
        if missing:
            problems.append(f"surface {index} missing fields: {', '.join(missing)}")
            continue
        row_id = cast(str, typed_row["id"])
        ids.append(row_id)
        state = cast(str, typed_row["state"])
        if state not in {"retained", "migrating", "retiring", "generated", "reserved"}:
            problems.append(f"{row_id}: invalid state {state!r}")
        for field in ("current", "targets", "lint", "tests", "ci"):
            value = typed_row[field]
            if not isinstance(value, list) or (field in {"lint", "tests", "ci"} and not value):
                problems.append(f"{row_id}: {field} must be a non-empty array")
        if not typed_row["coverage"] or not typed_row["debt"] or not typed_row["reason"]:
            problems.append(f"{row_id}: coverage, debt, and reason must be explicit")
        ci = cast(list[str], typed_row["ci"])
        lint = cast(list[str], typed_row["lint"])
        unknown_jobs = set(ci) - KNOWN_CI_JOBS
        if unknown_jobs:
            problems.append(f"{row_id}: unknown CI jobs: {sorted(unknown_jobs)}")
        unknown_lint = set(lint) - known_lint_owners
        if state != "reserved" and unknown_lint:
            problems.append(f"{row_id}: unknown lint owners: {sorted(unknown_lint)}")
        if state == "reserved" and not all(
            owner.startswith("next SDK sprint must install") for owner in lint
        ):
            problems.append(f"{row_id}: reserved lint owner must name the next sprint gate")
        targets.extend(cast(list[str], typed_row["targets"]))

    duplicate_ids = sorted({row_id for row_id in ids if ids.count(row_id) > 1})
    if duplicate_ids:
        problems.append(f"duplicate surface ids: {duplicate_ids}")

    declared_targets = set(targets)
    missing_targets = sorted(EXPECTED_TARGETS - declared_targets)
    extra_targets = sorted(declared_targets - EXPECTED_TARGETS)
    if missing_targets:
        problems.append(f"undeclared target surfaces: {missing_targets}")
    if extra_targets:
        problems.append(f"unknown target surfaces: {extra_targets}")

    complete_rows = [
        cast(dict[str, Any], row)
        for row in rows
        if isinstance(row, dict) and not any(field not in row for field in REQUIRED_FIELDS)
    ]
    for path in tracked:
        owners = [
            cast(str, row["id"])
            for row in complete_rows
            if any(
                _matches(path, declaration)
                for declaration in cast(list[str], row["current"])
            )
        ]
        if len(owners) != 1:
            problems.append(f"{path}: expected exactly one current owner, found {owners}")

    sdk_paths = [path for path in tracked if _matches(path, "sdk/")]
    if sdk_paths:
        problems.append(f"sdk/ is reserved for the next sprint but tracks: {sdk_paths}")
    return sorted(set(problems))


def _tracked_paths() -> list[str]:
    paths = subprocess.run(
        ("git", "ls-files"),
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    return [path for path in paths if (ROOT / path).exists()]


def test_undeclared_target_surface_fails_closed() -> None:
    policy = {"surface": [{
        "id": "only",
        "state": "retained",
        "current": ["README.md"],
        "targets": ["<root-files>"],
        "lint": ["fast.audit.markdown"],
        "tests": ["tests/citadel"],
        "ci": ["fast-gate", "pr-gate"],
        "coverage": "not-applicable: documentation",
        "debt": "none",
        "reason": "fixture",
    }]}
    assert any("undeclared target surfaces" in problem for problem in _problems(policy, ["README.md"]))


def test_duplicate_current_owner_fails_closed() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    duplicate = dict(policy["surface"][0])
    duplicate["id"] = "duplicate"
    policy["surface"].append(duplicate)
    assert any("expected exactly one current owner" in problem for problem in _problems(policy, _tracked_paths()))


def test_reserved_sdk_rejects_its_first_tracked_file() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    problems = _problems(policy, [*_tracked_paths(), "sdk/README.md"])
    assert any("sdk/ is reserved" in problem for problem in problems)


def test_missing_owner_dimension_fails_closed() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    del policy["surface"][0]["coverage"]
    assert any("missing fields: coverage" in problem for problem in _problems(policy, _tracked_paths()))


def test_unknown_lint_owner_fails_closed() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    policy["surface"][0]["lint"] = ["imaginary.checker"]
    assert any("unknown lint owners" in problem for problem in _problems(policy, _tracked_paths()))


def test_repository_surface_ownership_is_complete() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    problems = _problems(policy, _tracked_paths())
    assert not problems, OWNERSHIP_RATIONALE + "\n" + "\n".join(problems)


@pytest.mark.parametrize("row", tomllib.loads(POLICY.read_text(encoding="utf-8"))["surface"], ids=lambda row: row["id"])
def test_every_owner_names_executable_evidence(row: Mapping[str, Any]) -> None:
    assert all(value and "TBD" not in value for field in ("lint", "tests", "ci") for value in row[field])
    assert "TBD" not in row["coverage"] and "TBD" not in row["debt"] and "TBD" not in row["reason"]
