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
import subprocess
from pathlib import Path
from typing import NamedTuple

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]
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
    *sorted((PROJECT_ROOT / "build_system" / "builder" / "gate").rglob("*.py")),
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


# ---------------------------------------------------------------------------
# The third half: a reader that can only recognise a retired format.
#
# The two guards above stop a retired version being *stated* or *constructed*.
# Neither noticed a pattern that requires one. `prune-benchmark-history.py`
# matched `<series>_<major>.<minor>.<timestamp>` with the timestamp written
# `\d{6,}`, from the abolished `1.5.1783712334` scheme. Semver never has six
# digits there, so from the first semver release onward every recording fell
# through to "a shape
# we do not recognise" and became immortal. The policy the script documents had
# quietly stopped applying to the tree it was written to bound, and its own
# tests passed throughout, because they were written in the retired format too.
#
# A stranded reader is silent by construction: it does not fail, it stops
# matching. So this checks the shape of the pattern rather than any output --
# and reads it with the regex parser, because "does this require a long run of
# digits" is a question about structure, not about the characters someone typed.
# ---------------------------------------------------------------------------

#: A version component this long cannot be semver. The retired scheme's Unix
#: timestamp is ten digits and its date-plus-counter form is eight; a patch
#: number reaching six digits is not a thing that happens.
_TIMESTAMP_DIGITS = 6


def _is_digit_class(body) -> bool:
    """`\\d` or `[0-9]`, and nothing else in the class."""
    if len(body) != 1:
        return False
    operator, argument = body[0]
    if str(operator) != "IN":
        return False
    return all(
        (str(kind) == "CATEGORY" and str(value) == "CATEGORY_DIGIT")
        or (str(kind) == "RANGE" and value == (48, 57))
        for kind, value in argument
    )


class _Atom(NamedTuple):
    """One element of a flattened pattern.

    A named tuple rather than `tuple[str, object]`: the second field was read
    as both a repeat count and a literal character, so every use had to be
    narrowed and `>=` on it was a type error.
    """

    kind: str
    digits: int = 0
    literal: str = ""


def _atoms(node, out: list[_Atom]) -> None:
    """Flatten a parsed pattern into digit runs, literal characters, and rest.

    Enough structure to ask where a run of digits sits, and no more.
    """
    for operator, argument in node:
        name = str(operator)
        if name in {"MAX_REPEAT", "MIN_REPEAT"}:
            minimum, _, body = argument
            if _is_digit_class(body):
                out.append(_Atom("digits", digits=minimum))
            else:
                _atoms(body, out)
        elif name == "IN" and _is_digit_class([(operator, argument)]):
            out.append(_Atom("digits", digits=1))
        elif name == "LITERAL":
            out.append(_Atom("literal", literal=chr(argument)))
        elif name == "SUBPATTERN":
            _atoms(argument[3], out)
        elif name == "BRANCH":
            for branch in argument[1]:
                _atoms(branch, out)
        else:
            out.append(_Atom("other"))


def _requires_a_long_digit_run(pattern: str) -> bool:
    """Does this pattern require a six-digit *version component*?

    Parsed with `re`'s own parser. Asking the question of the pattern's source
    text means re-implementing regex syntax to answer it, which is the mistake
    that produced most of the guards in this repository.

    Position is half the question. `[0-9]{8}T[0-9]{6}Z` is an ISO timestamp
    and is none of this guard's business; the same eight digits after two
    dotted numbers is the retired scheme. The first version of this checked
    only the length and flagged the timestamp -- which would have taught the
    next person to widen the guard rather than fix a version.
    """
    import re as _re

    try:
        parsed = _re._parser.parse(pattern)
    except _re.error:
        return False

    atoms: list[_Atom] = []
    _atoms(parsed, atoms)
    for index in range(len(atoms) - 4):
        window = atoms[index : index + 5]
        if [atom.kind for atom in window] != [
            "digits",
            "literal",
            "digits",
            "literal",
            "digits",
        ]:
            continue
        if window[1].literal != "." or window[3].literal != ".":
            continue
        if window[4].digits >= _TIMESTAMP_DIGITS:
            return True
    return False


def _compiled_patterns(tree: ast.AST) -> list[str]:
    """Every literal pattern handed to `re.compile` in one module."""
    found = []
    for node in ast.walk(tree):
        if (
            isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and node.func.attr in {"compile", "match", "search", "fullmatch", "findall"}
            and node.args
            and isinstance(node.args[0], ast.Constant)
            and isinstance(node.args[0].value, str)
        ):
            found.append(node.args[0].value)
    return found


def test_no_reader_is_stranded_on_a_retired_version_format() -> None:
    """A pattern requiring six consecutive digits can only read a dead scheme.

    Not a style rule: such a pattern does not fail when the format moves, it
    stops matching. `prune-benchmark-history.py` went on reporting a bounded
    history while every new recording became permanent.
    """
    stranded = []
    for path in _stamping_and_tool_sources():
        try:
            tree = ast.parse(path.read_text(encoding="utf-8"))
        except SyntaxError:
            continue
        for pattern in _compiled_patterns(tree):
            if _requires_a_long_digit_run(pattern):
                stranded.append(f"{path.relative_to(PROJECT_ROOT)}: {pattern}")

    assert not stranded, (
        "these patterns require a run of six or more digits, which only the "
        "retired timestamp and date-counter schemes have. Against semver they "
        "match nothing and say nothing:\n  " + "\n  ".join(stranded)
    )


def _stamping_and_tool_sources() -> list[Path]:
    listed = subprocess.run(
        ["git", "ls-files", "-z", "--", "scripts", "src", "tests"],
        cwd=PROJECT_ROOT,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    return [
        PROJECT_ROOT / raw.decode()
        for raw in listed
        if raw
        and raw.decode().endswith(".py")
        and Path(raw.decode()).name != Path(__file__).name
        and (PROJECT_ROOT / raw.decode()).is_file()
    ]


# The guard's own tests: it must fire on the pattern that shipped, and stay
# quiet on the one that replaced it.


@pytest.mark.parametrize(
    "pattern",
    [
        r"^(?P<series>.+?)_(?P<major>\d+)\.(?P<minor>\d+)\.(?P<ts>\d{6,})",
        r"data_\d+\.\d+\.[0-9]{10}",
        r"v\d+\.\d+\.(?:\d{8,})",
    ],
)
def test_the_shipped_pattern_is_recognised_as_stranded(pattern: str) -> None:
    assert _requires_a_long_digit_run(pattern)


@pytest.mark.parametrize(
    "pattern",
    [
        r"^(?P<series>.+?)_(?P<major>\d+)\.(?P<minor>\d+)\.(?P<ts>\d+)",
        r"\d+\.\d+\.\d+",
        r"[0-9a-f]{40}",          # a git sha is not a version
        r"\w{6,}",                # six of something else is not six digits
    ],
)
def test_a_pattern_that_reads_semver_is_left_alone(pattern: str) -> None:
    assert not _requires_a_long_digit_run(pattern)


def test_a_timestamp_that_is_not_a_version_is_left_alone() -> None:
    """`[0-9]{8}T[0-9]{6}Z` is an ISO instant and none of this guard's business.

    The first version of this checked only the length of the digit run and
    flagged it. A guard that fires on correct code teaches the next person to
    widen the guard rather than fix the version, which is how a rule stops
    being one.
    """
    assert not _requires_a_long_digit_run(r"[0-9]{8}T[0-9]{6}Z")


def test_the_pruner_that_shipped_would_have_been_caught() -> None:
    """The exact pattern, from the file it shipped in.

    It required a six-digit patch, so under semver it matched nothing and
    every recording became permanent -- silently, because a stranded reader
    does not fail, it stops matching.
    """
    shipped = r"^(?P<series>.+?)_(?P<major>\d+)\.(?P<minor>\d+)\.(?P<ts>\d{6,})(?:_(?P<arch>[\w-]+))?\.json$"
    assert _requires_a_long_digit_run(shipped)

    import re as _re

    assert not _re.match(shipped, "data_1.2.3_x86_64.json"), (
        "the shipped pattern must not match semver; that is the whole bug"
    )
