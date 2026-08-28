"""Guard the direct, independently installable Python build-system project.

The migration has several shapes that can look healthy from inside an existing
checkout: root project metadata, a second source-directory layer, the old
distribution/import identity, or an entrypoint supplied by an ambient editable
install. This guard keeps the temporary legacy state exact and rejects every
new occurrence until T2 removes the debt.
"""

from __future__ import annotations

import hashlib
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

ROOT = Path(__file__).resolve().parents[2]
POLICY = Path(__file__).with_name("python_boundary_debt.toml")
SELF = "tests/citadel/test_python_project_boundary.py"

RATIONALE = """\
The build system must be one project rooted at build_system/, with the direct
builder/ source directory mapped to the installed capsem_builder namespace.
An ambient checkout, stale bytecode, old capsem identity, or undeclared Python
root can otherwise make local commands pass while clean installs fail.
"""

OLD_IMPORT = re.compile(r"^\s*(?:from|import)\s+(capsem(?:\.[A-Za-z_][A-Za-z0-9_.]*)?)", re.MULTILINE)
OBSOLETE_ROOTS = ("pyproject.toml", "uv.lock", "src/capsem/")
FORBIDDEN_NESTING = ("build_system/builder/src/", "build_system/builder/capsem_builder/")


@dataclass(frozen=True)
class Observed:
    obsolete_root_paths: tuple[str, ...]
    distributions: tuple[str, ...]
    entrypoints: tuple[str, ...]
    old_import_count: int
    old_import_sha256: str
    python_source_roots: tuple[str, ...]
    forbidden_nested_paths: tuple[str, ...]
    tracked_bytecode: tuple[str, ...]


def _tracked(root: Path = ROOT) -> list[str]:
    return subprocess.run(
        ("git", "ls-files"),
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()


def _project_records(root: Path, tracked: list[str]) -> tuple[list[str], list[str]]:
    distributions: list[str] = []
    entrypoints: list[str] = []
    for path in tracked:
        if not path.endswith("pyproject.toml"):
            continue
        project = tomllib.loads((root / path).read_text(encoding="utf-8")).get("project", {})
        name = project.get("name")
        if isinstance(name, str):
            distributions.append(f"{path}={name}")
        scripts = project.get("scripts", {})
        if isinstance(scripts, dict):
            entrypoints.extend(
                f"{path}:{command}={target}"
                for command, target in scripts.items()
                if isinstance(command, str) and isinstance(target, str)
            )
    return sorted(distributions), sorted(entrypoints)


def _old_import_inventory(root: Path, tracked: list[str]) -> tuple[int, str]:
    records: list[str] = []
    for path in tracked:
        if not path.endswith(".py") or path == SELF:
            continue
        text = (root / path).read_text(encoding="utf-8")
        records.extend(f"{path}\0{match.group(1)}" for match in OLD_IMPORT.finditer(text))
    payload = "\0".join(sorted(records)).encode()
    return len(records), hashlib.sha256(payload).hexdigest()


def _observe(root: Path = ROOT, tracked: list[str] | None = None) -> Observed:
    paths = tracked if tracked is not None else _tracked(root)
    path_set = set(paths)
    obsolete = [path for path in OBSOLETE_ROOTS[:2] if path in path_set]
    if any(path.startswith(OBSOLETE_ROOTS[2]) for path in paths):
        obsolete.append(OBSOLETE_ROOTS[2])
    distributions, entrypoints = _project_records(root, paths)
    old_import_count, old_import_sha256 = _old_import_inventory(root, paths)
    python_roots = sorted({path.split("/", 1)[0] for path in paths if path.endswith(".py")})
    nested = sorted(
        path for path in paths if any(path.startswith(prefix) for prefix in FORBIDDEN_NESTING)
    )
    bytecode = sorted(
        path for path in paths if path.endswith((".pyc", ".pyo")) or "/__pycache__/" in path
    )
    return Observed(
        obsolete_root_paths=tuple(obsolete),
        distributions=tuple(distributions),
        entrypoints=tuple(entrypoints),
        old_import_count=old_import_count,
        old_import_sha256=old_import_sha256,
        python_source_roots=tuple(python_roots),
        forbidden_nested_paths=tuple(nested),
        tracked_bytecode=tuple(bytecode),
    )


def _problems(observed: Observed, policy: dict[str, Any]) -> list[str]:
    problems: list[str] = []
    for field in (
        "obsolete_root_paths",
        "distributions",
        "entrypoints",
        "python_source_roots",
    ):
        actual = list(getattr(observed, field))
        expected = policy.get(field)
        if actual != expected:
            problems.append(f"{field}: expected {expected!r}, found {actual!r}")
    for field in ("old_import_count", "old_import_sha256"):
        actual = getattr(observed, field)
        expected = policy.get(field)
        if actual != expected:
            problems.append(f"{field}: expected {expected!r}, found {actual!r}")
    if observed.forbidden_nested_paths:
        problems.append(f"nested source-package paths: {list(observed.forbidden_nested_paths)!r}")
    if observed.tracked_bytecode:
        problems.append(f"tracked bytecode: {list(observed.tracked_bytecode)!r}")
    return problems


def _synthetic(**changes: object) -> Observed:
    values: dict[str, object] = {
        "obsolete_root_paths": (),
        "distributions": (),
        "entrypoints": (),
        "old_import_count": 0,
        "old_import_sha256": hashlib.sha256(b"").hexdigest(),
        "python_source_roots": (),
        "forbidden_nested_paths": (),
        "tracked_bytecode": (),
    }
    values.update(changes)
    return Observed(**values)  # type: ignore[arg-type]


@pytest.mark.parametrize(
    ("observed", "message"),
    [
        (_synthetic(obsolete_root_paths=("pyproject.toml",)), "obsolete_root_paths"),
        (_synthetic(distributions=("pyproject.toml=capsem",)), "distributions"),
        (
            _synthetic(entrypoints=("pyproject.toml:capsem-gate=capsem.gatelaunch:main",)),
            "entrypoints",
        ),
        (_synthetic(old_import_count=1, old_import_sha256="legacy"), "old_import_count"),
        (_synthetic(python_source_roots=("rogue",)), "python_source_roots"),
        (
            _synthetic(forbidden_nested_paths=("build_system/builder/src/pkg.py",)),
            "nested source-package paths",
        ),
        (_synthetic(tracked_bytecode=("build_system/builder/gate.pyc",)), "tracked bytecode"),
    ],
)
def test_each_prohibited_shape_is_observed_red(observed: Observed, message: str) -> None:
    empty_policy = {
        "obsolete_root_paths": [],
        "distributions": [],
        "entrypoints": [],
        "old_import_count": 0,
        "old_import_sha256": hashlib.sha256(b"").hexdigest(),
        "python_source_roots": [],
    }
    assert any(message in problem for problem in _problems(observed, empty_policy)), RATIONALE


def test_checkout_is_not_an_ambient_import_source() -> None:
    environment = {key: value for key, value in os.environ.items() if key != "PYTHONPATH"}
    result = subprocess.run(
        (sys.executable, "-I", "-S", "-c", "import capsem"),
        cwd=ROOT,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0, RATIONALE + "\ncapsem imported without an installed distribution"
    assert "ModuleNotFoundError" in result.stderr, result.stderr


def test_current_python_boundary_debt_is_exact() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    assert policy.pop("version") == 1
    problems = _problems(_observe(), policy)
    assert not problems, RATIONALE + "\n" + "\n".join(problems)
