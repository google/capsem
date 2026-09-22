"""Citadel guard: shared test helpers never import the bare name `conftest`.

Every suite with its own conftest.py (capsem-e2e, capsem-cli, capsem-service,
capsem-gateway, ...) makes `conftest` mean that suite's file. A helper that
imported the failure registry from `conftest` got the wrong module there and,
behind an `except ImportError: return`, preserved no evidence for any failing
VM test in those suites. A concurrency bug then left no logs to read.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

HELPERS = Path(__file__).resolve().parents[1] / "helpers"

HELPERS_CONFTEST_RATIONALE = """\
Shared test state lives in a helpers module (helpers.failures), never in a
conftest a helper imports by name: `conftest` resolves to whichever suite's
conftest.py is loaded, so the import silently reaches the wrong module.
"""


def _imports_conftest(source: str) -> bool:
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.ImportFrom) and node.module == "conftest" and not node.level:
            return True
        if isinstance(node, ast.Import) and any(alias.name == "conftest" for alias in node.names):
            return True
    return False


def test_no_helper_imports_conftest() -> None:
    offenders = [
        str(path.relative_to(HELPERS.parent))
        for path in sorted(HELPERS.rglob("*.py"))
        if _imports_conftest(path.read_text(encoding="utf-8"))
    ]
    assert not offenders, HELPERS_CONFTEST_RATIONALE + f"\nhelpers importing conftest: {offenders}"


@pytest.mark.parametrize(
    "source",
    [
        "from conftest import FAILED_NODEIDS\n",
        "def f():\n    from conftest import ARTIFACTS_ROOT\n",
        "import conftest\n",
        "import os, conftest as c\n",
    ],
)
def test_a_conftest_import_is_caught(source: str) -> None:
    assert _imports_conftest(source), HELPERS_CONFTEST_RATIONALE


@pytest.mark.parametrize(
    "source",
    ["from . import failures\n", "from helpers.failures import FAILED_NODEIDS\n", "import conftest_tools\n"],
)
def test_the_registry_module_passes(source: str) -> None:
    assert not _imports_conftest(source), HELPERS_CONFTEST_RATIONALE
