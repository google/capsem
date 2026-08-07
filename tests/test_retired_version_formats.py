"""No fixture may carry, and no script may construct, an abolished version.

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

Two halves, because scanning only for literals missed the thing that mattered.
A fixture states a version; the *stamper* builds one. `just _stamp-version`
went on assembling `1.${RELEASE_MINOR}.$(date +%s)` for the whole rewrite,
because a literal scan of Python cannot see a shell template in a justfile.
Nothing prevented reading that file -- the guard simply never looked. The
second test looks, at every file that participates in stamping, in whatever
language it is written in.

The literal half is scanned through the AST, over string literals only.
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

# Files that can stamp or assemble a version, in any language.
STAMPING_SOURCES = (
    PROJECT_ROOT / "justfile",
    *sorted((PROJECT_ROOT / "scripts").rglob("*.py")),
    *sorted((PROJECT_ROOT / "scripts").rglob("*.sh")),
    *sorted((PROJECT_ROOT / ".github" / "workflows").rglob("*.yaml")),
    # The stamping logic moved out of the justfile and into this package. A
    # guard that keeps scanning only where the code used to live protects an
    # empty recipe body.
    *sorted((PROJECT_ROOT / "src" / "capsem" / "gate").rglob("*.py")),
)

# A version component sourced from a clock. The left side is what makes it a
# *version*: a digit or the `}` closing an interpolated component, then the dot
# separating it from the component being appended.
CLOCK_COMPONENT = {
    "a shell clock": re.compile(r"[\d}]\.\$\(\(?\s*date\b"),
    "a Python clock": re.compile(
        r"[\d}]\.\{[^}]*\b(?:time\.time|datetime\b|utcnow|strftime)"
    ),
    "a dotted-date version": re.compile(r"date\s+[\"']?\+%Y[.\-]%m"),
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


def test_no_script_builds_a_version_component_from_a_clock() -> None:
    """A version says what changed. A clock says only when someone ran a build.

    Read as raw text on purpose: the offender here is a shell template inside a
    justfile, which has no AST to walk and no string literal to inspect.
    """
    offenders: list[str] = []
    scanned = 0
    for path in STAMPING_SOURCES:
        if not path.is_file() or path.name == Path(__file__).name:
            continue
        scanned += 1
        text = path.read_text(encoding="utf-8", errors="ignore")
        for line_number, line in enumerate(text.splitlines(), start=1):
            if line.lstrip().startswith("#"):
                continue
            for description, pattern in CLOCK_COMPONENT.items():
                if pattern.search(line):
                    offenders.append(
                        f"{path.relative_to(PROJECT_ROOT)}:{line_number} builds a "
                        f"version component from {description}: {line.strip()}"
                    )

    assert scanned > 10, "scanned too few files to trust this guard"
    assert not offenders, (
        "a version component came from a clock; semver components are chosen "
        "deliberately, and a timestamp cannot say whether a release is a fix "
        "or a feature:\n  " + "\n  ".join(offenders)
    )
