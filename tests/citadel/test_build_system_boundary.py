"""Keep the build-system migration exact while old roots shrink to zero."""

from __future__ import annotations

import hashlib
import re
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

ROOT = Path(__file__).resolve().parents[2]
POLICY = Path(__file__).with_name("build_system_boundary_debt.toml")
CONFIG = ROOT / "config" / "gate.toml"

RATIONALE = """\
Host build behavior belongs under build_system/. Root scripts/, docker/, and
release-site/ are exact migration debt, as is audit configuration still at the
repository root. Gate configuration must render only the owning new
paths after each move. This inventory may shrink or have keys rewritten to their
approved destination; it may never grow or be reset. See the T3 section of the
approved repository cleanup proposal and [boundary.scripts] in config/gate.toml.
"""

MIGRATING_ROOT_FILES = ("audit.toml",)
LEGACY_PLAN_PATH = re.compile(
    r"(?:^|[\s'\"])(?:scripts|docker|release-site)/|(?:^|/)entitlements[.]plist$"
)


@dataclass(frozen=True)
class Observed:
    legacy_programs: tuple[str, ...]
    root_dockerfiles: tuple[str, ...]
    release_site: tuple[str, ...]
    root_package_surfaces: tuple[str, ...]
    plan_paths: tuple[str, ...]


def _git(*args: str) -> list[str]:
    result = subprocess.run(
        ("git", *args),
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr)
    return result.stdout.splitlines()


def _digest(records: tuple[str, ...]) -> str:
    return hashlib.sha256("\0".join(sorted(records)).encode()).hexdigest()


def _tracked_programs() -> tuple[str, ...]:
    programs: list[str] = []
    for path in _git("ls-files", "scripts"):
        candidate = ROOT / path
        if candidate.is_symlink() or not candidate.is_file():
            continue
        if candidate.suffix in {".py", ".sh"} or candidate.read_bytes().startswith(b"#!"):
            programs.append(path)
    return tuple(sorted(programs))


def _walk_strings(value: Any, key: str = "") -> list[tuple[str, str]]:
    if isinstance(value, dict):
        return [
            pair
            for child, child_value in value.items()
            for pair in _walk_strings(child_value, f"{key}.{child}".lstrip("."))
        ]
    if isinstance(value, list):
        return [
            pair
            for index, child_value in enumerate(value)
            for pair in _walk_strings(child_value, f"{key}[{index}]")
        ]
    if isinstance(value, str):
        return [(key, value)]
    return []


def _plan_paths() -> tuple[str, ...]:
    config = tomllib.loads(CONFIG.read_text(encoding="utf-8"))
    return tuple(
        sorted(
            f"{key}={value}"
            for key, value in _walk_strings(config)
            if LEGACY_PLAN_PATH.search(value)
        )
    )


def _observe() -> Observed:
    tracked = tuple(_git("ls-files"))
    return Observed(
        legacy_programs=_tracked_programs(),
        root_dockerfiles=tuple(
            path for path in tracked if path.startswith("docker/Dockerfile")
        ),
        release_site=tuple(path for path in tracked if path.startswith("release-site/")),
        root_package_surfaces=tuple(path for path in MIGRATING_ROOT_FILES if path in tracked),
        plan_paths=_plan_paths(),
    )


def _problems(policy: dict[str, Any], observed: Observed) -> list[str]:
    problems: list[str] = []
    for field, records in (
        ("legacy_program", observed.legacy_programs),
        ("root_dockerfile", observed.root_dockerfiles),
        ("release_site", observed.release_site),
        ("root_package_surface", observed.root_package_surfaces),
        ("plan_path", observed.plan_paths),
    ):
        expected_count = policy.get(f"{field}_count")
        expected_digest = policy.get(f"{field}_sha256")
        found_digest = _digest(records)
        if len(records) != expected_count or found_digest != expected_digest:
            problems.append(
                f"{field} debt: expected count={expected_count!r} "
                f"sha256={expected_digest!r}; found count={len(records)} "
                f"sha256={found_digest}"
            )
    return problems


def _synthetic(**changes: tuple[str, ...]) -> Observed:
    values = {
        "legacy_programs": (),
        "root_dockerfiles": (),
        "release_site": (),
        "root_package_surfaces": (),
        "plan_paths": (),
    }
    values.update(changes)
    return Observed(**values)


def _empty_policy() -> dict[str, Any]:
    empty = hashlib.sha256(b"").hexdigest()
    return {
        f"{field}_{suffix}": 0 if suffix == "count" else empty
        for field in (
            "legacy_program",
            "root_dockerfile",
            "release_site",
            "root_package_surface",
            "plan_path",
        )
        for suffix in ("count", "sha256")
    }


@pytest.mark.parametrize(
    ("observed", "message"),
    [
        (_synthetic(legacy_programs=("scripts/new.py",)), "legacy_program debt"),
        (
            _synthetic(root_dockerfiles=("docker/Dockerfile.new",)),
            "root_dockerfile debt",
        ),
        (_synthetic(release_site=("release-site/new.ts",)), "release_site debt"),
        (
            _synthetic(root_package_surfaces=("entitlements.plist",)),
            "root_package_surface debt",
        ),
        (
            _synthetic(plan_paths=("build.script=scripts/new.py",)),
            "plan_path debt",
        ),
    ],
)
def test_each_old_build_surface_is_observed_red(
    observed: Observed, message: str
) -> None:
    assert any(message in problem for problem in _problems(_empty_policy(), observed)), (
        RATIONALE
    )


def test_missing_debt_policy_fails_closed() -> None:
    assert len(_problems({}, _synthetic())) == 5, RATIONALE


def test_current_build_system_boundary_is_exact() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    assert policy.get("version") == 1
    problems = _problems(policy, _observe())
    assert not problems, RATIONALE + "\n" + "\n".join(problems)
