"""Release contracts parse workflow and shell structure with owned parsers.

The binary replay contract matched ``echo NAME=`` until the workflow switched
to ``printf``. The paired-content guard sliced YAML jobs by indentation and
therefore never followed the profile job into its dispatched script. Both
looked fast and precise; both were second, incomplete grammars that made the
release dispatcher the first honest test.
"""

from __future__ import annotations

import ast
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RELEASE_CONTRACTS = ROOT / "tests/capsem-release"
WORKFLOW_GUARDS = (
    ROOT / "tests/citadel/test_ci_calls_only_public_recipes.py",
    ROOT / "tests/citadel/test_paired_content_reaches_every_lane.py",
    ROOT / "tests/citadel/test_workflow_enforcement.py",
    ROOT / "tests/citadel/test_workflow_heredocs_do_not_run_their_backticks.py",
)


def _imports_regex(source: str, *, filename: str = "<guard probe>") -> bool:
    tree = ast.parse(source, filename=filename)
    return any(
        (isinstance(node, ast.Import) and any(alias.name == "re" for alias in node.names))
        or (isinstance(node, ast.ImportFrom) and node.module == "re")
        for node in ast.walk(tree)
    )


def test_release_shell_contracts_cannot_grow_a_regex_parser() -> None:
    watched = (*sorted(RELEASE_CONTRACTS.glob("*.py")), *WORKFLOW_GUARDS)
    assert watched and all(path.is_file() for path in watched)

    offenders = [
        str(path.relative_to(ROOT))
        for path in watched
        if _imports_regex(path.read_text(encoding="utf-8"), filename=str(path))
    ]

    assert not offenders, (
        "release/workflow contracts imported regex and can now invent a second "
        "shell or YAML grammar. Select jobs and steps through PyYAML, extract "
        "run bodies with shellsurfaces, and ask shelllex/shellparse about "
        "commands, assignments and heredocs:\n  "
        + "\n  ".join(offenders)
    )


def test_the_guard_detects_a_regex_import() -> None:
    assert _imports_regex("import re\n")
