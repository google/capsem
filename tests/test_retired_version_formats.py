"""No fixture may teach a version format the project abolished.

Capsem versions are semver: the binary's patch increments, and every profile
revision is MAJOR.MINOR.PATCH. Two formats were retired:

    1.6.1785421421      a Unix timestamp in the patch, so a compatibility
                        window could only say "built before/after this instant"
    2026.06.08.9        a date plus an edit counter, where the date recorded
                        when a human last typed it and the counter counted
                        edits rather than publications

Fixtures carrying them keep the dead schemes alive: they read as the current
format to anyone learning from the tests, and they break in a body when the
real scheme moves, reporting a broken release rather than a stale fixture.

Scanned through the AST rather than the raw text, over string literals only.
Docstrings and comments are exempt because they must be able to *name* a
retired format in order to warn about it -- a raw-text scan flagged the correct
term "profile pins" for containing the retired "file pins" earlier in the same
session, which would have pushed prose away from the right vocabulary.
"""

from __future__ import annotations

import ast
import re
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent
SEARCH_ROOTS = (PROJECT_ROOT / "tests", PROJECT_ROOT / "scripts")

RETIRED_FORMATS = {
    "a Unix timestamp in the patch": re.compile(r"^v?\d+\.\d+\.[12]\d{9}$"),
    "a dotted-date revision": re.compile(r"^\d{4}\.\d{2,4}\.\d+\.\d+$"),
}


def _docstrings(tree: ast.Module) -> set[int]:
    """Node ids of docstring constants, which may name a retired format."""
    exempt = set()
    for node in ast.walk(tree):
        if isinstance(
            node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)
        ):
            body = getattr(node, "body", [])
            if (
                body
                and isinstance(body[0], ast.Expr)
                and isinstance(body[0].value, ast.Constant)
                and isinstance(body[0].value.value, str)
            ):
                exempt.add(id(body[0].value))
    return exempt


def test_no_fixture_carries_a_retired_version_format() -> None:
    offenders: list[str] = []
    scanned = 0
    for root in SEARCH_ROOTS:
        for path in sorted(root.rglob("*.py")):
            if path.name == Path(__file__).name:
                continue
            try:
                tree = ast.parse(path.read_text(encoding="utf-8", errors="ignore"))
            except SyntaxError:
                continue
            scanned += 1
            exempt = _docstrings(tree)
            for node in ast.walk(tree):
                if not isinstance(node, ast.Constant) or not isinstance(node.value, str):
                    continue
                if id(node) in exempt:
                    continue
                for description, pattern in RETIRED_FORMATS.items():
                    if pattern.match(node.value):
                        offenders.append(
                            f"{path.relative_to(PROJECT_ROOT)}:{node.lineno} "
                            f"{node.value!r} uses {description}"
                        )

    assert scanned > 50, "scanned too few files to trust this guard"
    assert not offenders, (
        "these fixtures carry a retired version format, which teaches a dead "
        "scheme and breaks when the real one moves:\n  " + "\n  ".join(offenders)
    )
