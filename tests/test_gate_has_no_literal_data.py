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


# ---------------------------------------------------------------------------
# What the suffix list and the flat-string walk both miss
# ---------------------------------------------------------------------------
#
# Two shapes slipped past the checks above for as long as they existed.
#
# A path built with `/` is a `BinOp` chain, so every component is inspected as
# a separate string and none of them looks like a path:
#
#     Path(root) / "private" / "tauri" / "capsem.key"
#
# And an environment variable name is not a path at all, so nothing looked at
# it -- while `os.environ.get("CAPSEM_INSTALL_MANIFEST_URL")` is exactly the
# deployment data this file exists to keep in one place. The rail it selects
# cannot be renamed without finding every literal by hand.

#: Suffixes that name a file on disk, wherever they appear in a `/` chain.
PATH_SUFFIXES = (*FILE_SUFFIXES, ".key", ".txt", ".img", ".erofs", ".pkg", ".sock")

#: Reading these is how a module asks the environment a question.
ENVIRONMENT_READERS = {"getenv", "get", "environ"}

#: Standard process and tool conventions. These are not Capsem rails and
#: moving them into TOML would be dumping strings into a file rather than
#: giving a protocol an owner -- `HOME` means what it means everywhere.
CONVENTIONS = frozenset(
    {
        "HOME",
        "PATH",
        "TMPDIR",
        "USER",
        "LANG",
        "LC_ALL",
        "XDG_RUNTIME_DIR",
        "PKG_CONFIG_PATH",
        "UV_PROJECT_ENVIRONMENT",
        "RUST_LOG",
        "RUSTFLAGS",
        "CARGO_TARGET_DIR",
        "DOCKER_HOST",
        "GITHUB_ENV",
        "GITHUB_OUTPUT",
        "CI",
        "PYTHONPYCACHEPREFIX",
        "PYTHONDONTWRITEBYTECODE",
        "COLUMNS",
        "TERM",
        "HOST_UID",
        "HOST_GID",
    }
)


def _environment_writes(tree: ast.AST) -> list[str]:
    """Literal names used as keys of a dictionary handed to a process.

    A read was caught and a *write* was not, so the same rail could be named
    once through configuration and once as a dict key three modules along --
    invisible to the guard, and exactly as hard to rename.

    A dictionary whose keys are upper-case identifiers is an environment; no
    other dictionary in this package is spelled that way.
    """
    found: list[str] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Dict):
            continue
        for key in node.keys:
            if not isinstance(key, ast.Constant) or not isinstance(key.value, str):
                continue
            name = key.value
            if name.isupper() and name.replace("_", "").isalnum() and "_" in name:
                found.append(name)
    return found


def _path_chain_literals(tree: ast.AST) -> list[str]:
    """Literal components of any `x / "a" / "b"` expression."""
    found: list[str] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.BinOp) or not isinstance(node.op, ast.Div):
            continue
        for side in (node.left, node.right):
            if not isinstance(side, ast.Constant) or not isinstance(side.value, str):
                continue
            if side.value.endswith(PATH_SUFFIXES) or "/" in side.value:
                found.append(side.value)
    return found


def _environment_literals(tree: ast.AST) -> list[str]:
    """Literal variable names read straight out of the environment."""
    found: list[str] = []
    for node in ast.walk(tree):
        if (
            isinstance(node, ast.Subscript)
            and _is_environ(node.value)
            and isinstance(node.slice, ast.Constant)
            and isinstance(node.slice.value, str)
        ):
            found.append(node.slice.value)
        if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Attribute):
            continue
        if node.func.attr not in ENVIRONMENT_READERS:
            continue
        if not (_is_environ(node.func.value) or _is_os(node.func.value)):
            continue
        if not node.args or not isinstance(node.args[0], ast.Constant):
            continue
        value = node.args[0].value
        if isinstance(value, str) and value.isupper():
            found.append(value)
    return found


def _is_environ(node: ast.AST) -> bool:
    return isinstance(node, ast.Attribute) and node.attr == "environ"


def _is_os(node: ast.AST) -> bool:
    return isinstance(node, ast.Name) and node.id == "os"


@pytest.mark.parametrize("module", _gate_modules(), ids=lambda path: path.name)
def test_no_module_builds_a_path_out_of_literal_components(module: Path) -> None:
    allowed = BOOTSTRAP.get(module.name, set())
    literals = [
        value
        for value in _path_chain_literals(ast.parse(module.read_text(encoding="utf-8")))
        if value not in allowed
    ]

    assert not literals, (
        f"{module.name} builds a path from literal components {literals}; "
        "declare it in config/gate.toml so one place owns where it lives"
    )


@pytest.mark.parametrize("module", _gate_modules(), ids=lambda path: path.name)
def test_no_module_spells_an_environment_variable(module: Path) -> None:
    literals = _environment_literals(ast.parse(module.read_text(encoding="utf-8")))

    assert not literals, (
        f"{module.name} reads {literals} from the environment by name; "
        "declare the name in config/gate.toml so the rail can be renamed in "
        "one place"
    )


@pytest.mark.parametrize("module", _gate_modules(), ids=lambda path: path.name)
def test_no_module_names_an_environment_variable_it_writes(module: Path) -> None:
    """The other half of the same rule.

    `[environment]` owns `CAPSEM_HOME` and `CAPSEM_RUN_DIR`, and `assets` and
    `service` spelled both again as dictionary keys. The guard watched reads
    only, so the duplicates were invisible to the one thing meant to find
    them.
    """
    literals = sorted(
        {
            value
            for value in _environment_writes(ast.parse(module.read_text(encoding="utf-8")))
            if value not in CONVENTIONS
        }
    )

    assert not literals, (
        f"{module.name} builds an environment naming {literals}; declare the "
        "names in config/gate.toml and go through it, so the rail can be "
        "renamed in one place"
    )


def test_an_environment_write_would_be_caught(tmp_path: Path) -> None:
    module = tmp_path / "offender.py"
    module.write_text('x = {"CAPSEM_HOME": str(home), "PATH": "/usr/bin"}\n')

    assert _environment_writes(ast.parse(module.read_text())) == ["CAPSEM_HOME"]


def test_a_path_built_from_literals_would_be_caught(tmp_path: Path) -> None:
    """The guard, watched failing on the shape it exists for."""
    module = tmp_path / "offender.py"
    module.write_text('x = Path(root) / "private" / "tauri" / "capsem.key"\n')

    assert _path_chain_literals(ast.parse(module.read_text())) == ["capsem.key"]


def test_an_environment_read_would_be_caught(tmp_path: Path) -> None:
    module = tmp_path / "offender.py"
    module.write_text(
        'a = os.environ.get("CAPSEM_INSTALL_CHANNEL")\n'
        'b = os.environ["CAPSEM_HOME"]\n'
        'c = os.getenv("CAPSEM_RUN_DIR")\n'
    )

    assert sorted(_environment_literals(ast.parse(module.read_text()))) == [
        "CAPSEM_HOME",
        "CAPSEM_INSTALL_CHANNEL",
        "CAPSEM_RUN_DIR",
    ]


def test_a_composed_path_with_no_literal_is_not_flagged(tmp_path: Path) -> None:
    """Composition itself is fine; only spelling the destination is not."""
    module = tmp_path / "fine.py"
    module.write_text("x = Path(root) / settings.private / settings.key_name\n")

    assert _path_chain_literals(ast.parse(module.read_text())) == []
