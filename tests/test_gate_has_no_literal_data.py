"""No path, architecture, or channel is spelled in the gate's code.

The justfile's defining problem was not that it was shell. It was that the same
value lived in eleven places: `--boundary X --rail Y` written out at each call
site, four `case` blocks over architecture names each free to disagree, the
pinned Rust toolchain spelled three times inside one script. Moving that into
Python fixed nothing by itself -- the first draft of this package promptly grew
`CONTAINER = "capsem-install-test"` and `LAYOUT = Layout(assets="target/...")`
in whichever module happened to need them.

`config/gate.toml` is the answer, and this is what keeps it the answer. Three
rules, each aimed at a value that drifts silently rather than failing loudly:

**Paths and filenames.** A path spelled in code is a path that can disagree
with the one in config, and neither copy knows about the other. `versions.py`
really did carry its own copy of the stamped-file list.

**Architectures and channels.** `arm64`, `aarch64`, `amd64`, `stable`,
`nightly` -- the vocabulary that had four disagreeing `case` blocks. These come
from `config.arch(...)` and `config.package.channels`, which fail by name on a
value they do not recognise.

**Literal arguments to file calls.** `open("target/thing")` is the same defect
in the form the user meets it. The argument must be a value that came from
somewhere.

What is deliberately *not* forbidden: table keys. `release("after-install")`
and `ensure_space("install")` name which entry to look up, and an unknown one
raises immediately with the legal set listed. That is an API, not a copy.
"""

from __future__ import annotations

import ast
import re
import tomllib
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[1]
GATE_PACKAGE = PROJECT_ROOT / "src" / "capsem" / "gate"

# Calls whose first positional argument is a path. Split by call shape because
# the signatures differ: builtin `open(path, mode)` takes one first, while
# `Path.open(mode)` and `Path.read_text(encoding)` take none at all -- treating
# those the same flags a file mode as a hardcoded path.
PATH_FUNCTIONS = {"open", "Path"}
PATH_METHODS = {"rmtree", "copytree", "copy", "copyfile", "copy2"}

# Suffixes that make a bare word a filename.
FILE_SUFFIXES = (".toml", ".json", ".py", ".sh", ".yaml", ".yml", ".log", ".deb", ".lock")

# The bootstrap exemption, and only that: the loader has to name the file it
# loads, or there is nowhere for the first value to come from.
BOOTSTRAP = {
    "config.py": {"config", "gate.toml"},
    # The doctor reads pyproject to check the console scripts it declares --
    # that is the file it is auditing, not a path it works from.
    "doctor.py": {"pyproject.toml"},
}


def _gate_modules() -> list[Path]:
    modules = sorted(GATE_PACKAGE.glob("*.py"))
    assert len(modules) > 10, "scanned too few modules to trust this guard"
    return modules


def _config() -> dict:
    return tomllib.loads((PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))


def _vocabulary() -> set[str]:
    """Architecture spellings and channel names, from the tables that own them."""
    raw = _config()
    words = set(raw["package"]["channels"])
    for name, spec in raw["architectures"].items():
        words.add(name)
        words.update(spec["aliases"])
    return words


def _literals(module: Path) -> list[tuple[int, str]]:
    """Every string constant that is not a docstring."""
    tree = ast.parse(module.read_text(encoding="utf-8"))
    docstrings = {
        ast.get_docstring(node, clean=False)
        for node in ast.walk(tree)
        if isinstance(node, ast.Module | ast.ClassDef | ast.FunctionDef | ast.AsyncFunctionDef)
    }
    return [
        (node.lineno, node.value)
        for node in ast.walk(tree)
        if isinstance(node, ast.Constant)
        and isinstance(node.value, str)
        and node.value not in docstrings
    ]


# Shapes that contain a slash without being a path: prose in an error message,
# a regular expression, a format template whose parts already came from config.
PROSE = re.compile(r"\s")
REGEX = re.compile(r"[\\^$(\[*+?]")
BARE_SUFFIX = re.compile(r"^\.[a-z0-9]+$")
COMPOSED = re.compile(r"^[{}/\w.-]*\{[^}]+\}[{}/\w.-]*$")


def _looks_like_a_path(value: str) -> bool:
    """Whether this literal spells a filesystem location.

    A bare suffix like `.log` is a type marker rather than a path, and putting
    every file extension in config would be ceremony without a payoff. A
    template such as `{mount}/{relative}` joins two values that already came
    from somewhere.
    """
    if PROSE.search(value) or REGEX.search(value) or BARE_SUFFIX.match(value):
        return False
    if COMPOSED.match(value):
        return False
    if value.endswith(FILE_SUFFIXES):
        return True
    if "/" not in value:
        return False
    # A separator between two interpolated values names nothing: an f-string's
    # literal parts are punctuation, and the data is in the braces. A fragment
    # with a word still attached -- `/usr/bin/`, `:/src` -- does name something.
    return bool(value.strip("/:\"'"))


def _allowed(module: Path, value: str) -> bool:
    return value in BOOTSTRAP.get(module.name, set())


# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("module", _gate_modules(), ids=lambda p: p.name)
def test_no_module_spells_a_path(module: Path) -> None:
    """A path in code is a path that can disagree with the one in config.

    `versions.py` carried its own copy of the stamped-file list while
    `[[versions.stamped]]` declared the same files, and nothing connected them.
    """
    offenders = [
        f"{module.name}:{line}: {value!r}"
        for line, value in _literals(module)
        if _looks_like_a_path(value) and not _allowed(module, value)
    ]

    assert not offenders, (
        "these spell a path in code; take it from config/gate.toml so there is "
        "one copy:\n  " + "\n  ".join(offenders)
    )


@pytest.mark.parametrize("module", _gate_modules(), ids=lambda p: p.name)
def test_no_file_call_receives_a_literal(module: Path) -> None:
    """`open("target/thing")` is the same defect in the form you meet it."""
    tree = ast.parse(module.read_text(encoding="utf-8"))
    offenders = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        if isinstance(node.func, ast.Attribute):
            name = node.func.attr
            takes_a_path = name in PATH_METHODS
        elif isinstance(node.func, ast.Name):
            name = node.func.id
            takes_a_path = name in PATH_FUNCTIONS
        else:
            continue
        if not takes_a_path:
            continue
        # The first positional argument only: `open(path, "a")` takes a mode
        # second, and a mode is not a path.
        argument = node.args[0] if node.args else None
        if (
            isinstance(argument, ast.Constant)
            and isinstance(argument.value, str)
            and not _allowed(module, argument.value)
        ):
            offenders.append(f"{module.name}:{node.lineno}: {name}({argument.value!r})")

    assert not offenders, (
        "pass a value that came from config rather than a literal:\n  "
        + "\n  ".join(offenders)
    )


# ---------------------------------------------------------------------------
# Vocabulary
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("module", _gate_modules(), ids=lambda p: p.name)
def test_no_module_spells_an_architecture_or_channel(module: Path) -> None:
    """The vocabulary that had four disagreeing `case` blocks.

    `config.arch(...)` and `config.package.channels` fail by name on a value
    they do not recognise; a literal just quietly means something else.
    """
    words = _vocabulary()
    offenders = [
        f"{module.name}:{line}: {value!r}"
        for line, value in _literals(module)
        if value in words
    ]

    assert not offenders, (
        "resolve these through the config rather than naming them:\n  "
        + "\n  ".join(offenders)
    )


# ---------------------------------------------------------------------------
# The guard itself
# ---------------------------------------------------------------------------


def test_the_bootstrap_exemption_stays_minimal() -> None:
    """It exists so the loader can name the file it loads. Nothing else."""
    assert set(BOOTSTRAP) == {"config.py", "doctor.py"}
    assert sum(len(values) for values in BOOTSTRAP.values()) <= 3


def test_a_path_literal_would_be_caught(tmp_path: Path) -> None:
    """Red-first, permanently: the guard must see the shape it forbids."""
    module = tmp_path / "example.py"
    module.write_text('LAYOUT = "target/install-test-assets"\n')

    caught = [value for _line, value in _literals(module) if _looks_like_a_path(value)]

    assert caught == ["target/install-test-assets"]


def test_a_file_mode_is_not_mistaken_for_a_path(tmp_path: Path) -> None:
    """`Path.open("a")` takes a mode where builtin `open` takes a path."""
    module = tmp_path / "example.py"
    module.write_text('handle = destination.open("a", encoding="utf-8")\n')

    tree = ast.parse(module.read_text())
    methods = [
        node.func.attr
        for node in ast.walk(tree)
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
    ]

    assert "open" in methods
    assert "open" not in PATH_METHODS


def test_a_composed_path_is_not_flagged(tmp_path: Path) -> None:
    """`f"{mount}/{relative}"` joins two values that already came from config."""
    module = tmp_path / "example.py"
    module.write_text('SEPARATOR = "{mount}/{relative}"\n')

    caught = [value for _line, value in _literals(module) if _looks_like_a_path(value)]

    assert caught == []
