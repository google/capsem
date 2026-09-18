"""Citadel guard: a test service owns its session ledger.

The service keeps the main session ledger at `run_dir.parent/sessions/main.db`
(`main_db_path_for_run_dir`), matching the installed layout where CAPSEM_HOME
owns a `run/` directory. A harness that sets CAPSEM_RUN_DIR and CAPSEM_HOME to
the same temporary directory moves that ledger up into the run-wide temporary
parent, so every service started that way in one pytest run shares a single
main.db. The e2e harness did, and its exec tests failed only when other
workers ran beside it, never alone.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]

SERVICE_HOME_LAYOUT_RATIONALE = """\
A test service must use the installed home/run layout: CAPSEM_HOME owns a
run/ directory, and CAPSEM_RUN_DIR is that run/ directory
(helpers.service.make_service_home_run_dirs). Pointing both at one directory
puts sessions/main.db in the run-wide temporary parent, shared by every
parallel worker's service.
"""


def _shared_layouts(source: str) -> list[str]:
    """Assignments of CAPSEM_RUN_DIR and CAPSEM_HOME from the same expression."""
    values: dict[str, list[str]] = {}
    for node in ast.walk(ast.parse(source)):
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            continue
        target = node.targets[0]
        if not (isinstance(target, ast.Subscript) and isinstance(target.slice, ast.Constant)):
            continue
        key = target.slice.value
        if key in ("CAPSEM_RUN_DIR", "CAPSEM_HOME"):
            values.setdefault(key, []).append(ast.unparse(node.value))
    return sorted(set(values.get("CAPSEM_RUN_DIR", [])) & set(values.get("CAPSEM_HOME", [])))


def test_no_harness_collapses_home_and_run_dir() -> None:
    offenders = {
        str(path.relative_to(ROOT)): shared
        for path in sorted((ROOT / "tests").rglob("*.py"))
        if "citadel" not in path.parts
        for shared in [_shared_layouts(path.read_text(encoding="utf-8"))]
        if shared
    }
    assert not offenders, SERVICE_HOME_LAYOUT_RATIONALE + f"\nshared layouts: {offenders}"


@pytest.mark.parametrize(
    "source",
    [
        'env["CAPSEM_RUN_DIR"] = str(self.tmp_dir)\nenv["CAPSEM_HOME"] = str(self.tmp_dir)\n',
        'env["CAPSEM_HOME"] = str(d)\nx = 1\nenv["CAPSEM_RUN_DIR"] = str(d)\n',
        'environ["CAPSEM_RUN_DIR"] = path\nenviron["CAPSEM_HOME"] = path\n',
    ],
)
def test_a_collapsed_layout_is_caught(source: str) -> None:
    assert _shared_layouts(source), SERVICE_HOME_LAYOUT_RATIONALE


@pytest.mark.parametrize(
    "source",
    [
        'env["CAPSEM_RUN_DIR"] = str(self.tmp_dir)\nenv["CAPSEM_HOME"] = str(self.home_dir)\n',
        'env["CAPSEM_RUN_DIR"] = str(run)\n',
    ],
)
def test_the_installed_layout_passes(source: str) -> None:
    assert not _shared_layouts(source), SERVICE_HOME_LAYOUT_RATIONALE
