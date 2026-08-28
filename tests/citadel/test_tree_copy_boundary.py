"""Citadel guard: the gate never copies a tree through a symlink.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one destroyed a run log, and the destroyed log was well-formed
afterwards -- which is why nothing noticed.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
GATE = PROJECT_ROOT / "build_system" / "builder" / "gate"

#: The one module allowed to spell `shutil.copytree`. Everything else goes
#: through `copy_tree` / `merge_tree`, which is where the rule is enforced and
#: where the reasoning is written down.
OWNER = "filesystem.py"

TREE_COPY_RATIONALE = """\
Copying a tree must never follow a symlink, and only `filesystem` may decide
how.

`shutil.copytree(src, dst, dirs_exist_ok=True)` does two things its call sites
did not expect. It *dereferences* a symlink in the source and writes the
contents as a real directory under that name. And when the destination entry of
that name is itself a symlink, the write goes **through** it, into whatever it
points at.

Both halves were live in the prefix export, and together they destroyed run
logs. `target/gate-runs/latest` points at the newest run, on the host and
inside the private checkout alike. Exporting a run therefore read the private
tree's `latest` as a directory of files and wrote them through the host's
`latest` into an unrelated older run, replacing every file in it. `copytree`
copies with `copy2`, so the clobbered run kept the source's timestamps as well:
a perfectly well-formed log describing a run that never happened in it.

One `test-fast` run destroyed a `test-release-contracts` log this way, twice in
one hour. `source.verify` and the timing ratchet both read these logs as
evidence, so the corruption is not cosmetic -- and it is invisible, because the
result is valid JSONL with plausible timestamps.

`symlinks=True` is not the fix and a contract about that flag would have passed
a broken one: it stops the dereference and then raises `FileExistsError`
because the destination link is already there, failing every export. A
destination symlink has to be *replaced*.

So there is one implementation, in `capsem_builder.gate.filesystem`:

  copy_tree   replaces the target outright
  merge_tree  copies into an existing tree, never following a link either side

Reach for those. A second hand-rolled `copytree` is a second place for this
decision to be made differently, and the first one was made wrong.

See build_system/builder/gate/filesystem.py and tests/test_gate_prefix.py.
"""


def _calls(path: Path) -> list[ast.Call]:
    """Every `shutil.copytree(...)` call in a module.

    Parsed rather than grepped: `copytree` appears in prose in three docstrings
    in this package, including the one explaining why not to call it, and a
    guard that fails on its own explanation is a guard that gets deleted.
    """
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    return [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.Call)
        and isinstance(node.func, ast.Attribute)
        and node.func.attr == "copytree"
    ]


def _modules() -> list[Path]:
    return sorted(GATE.glob("*.py"))


def test_the_gate_has_modules_to_check() -> None:
    """A guard over an empty file list asserts nothing."""
    assert _modules(), "no gate modules found; this contract would be vacuous"


@pytest.mark.parametrize("module", _modules(), ids=lambda path: path.name)
def test_only_the_owner_spells_copytree(module: Path) -> None:
    if module.name == OWNER:
        return
    found = _calls(module)
    assert not found, (
        TREE_COPY_RATIONALE
        + f"\n{module.name} calls shutil.copytree at line(s) "
        + ", ".join(str(call.lineno) for call in found)
        + "; use capsem_builder.gate.filesystem.copy_tree or merge_tree instead"
    )


def test_the_owner_never_merges_through_copytree() -> None:
    """`dirs_exist_ok=True` is the shape that writes into an existing tree.

    That is the one `copytree` cannot be made safe with a flag, so `merge_tree`
    does it by hand. Replacing a target outright is a different case: nothing
    exists at the destination to be written through.
    """
    merging = [
        call
        for call in _calls(GATE / OWNER)
        if any(keyword.arg == "dirs_exist_ok" for keyword in call.keywords)
    ]
    assert not merging, (
        TREE_COPY_RATIONALE
        + f"\n{OWNER} merges with shutil.copytree at line(s) "
        + ", ".join(str(call.lineno) for call in merging)
    )


# -- adversarial: the guard has to see the shapes it claims to -------------


@pytest.mark.parametrize(
    "source",
    [
        "shutil.copytree(a, b)",
        "shutil.copytree(a, b, dirs_exist_ok=True)",
        "shutil.copytree(a, b, symlinks=True)",
        "x = shutil.copytree(a, b)",
        "if c:\n    shutil.copytree(a, b)",
    ],
)
def test_every_call_shape_is_seen(source: str, tmp_path: Path) -> None:
    module = tmp_path / "sample.py"
    module.write_text(source, encoding="utf-8")
    assert _calls(module), f"the guard cannot see {source!r}"


@pytest.mark.parametrize(
    "source",
    [
        '"""A docstring mentioning shutil.copytree by name."""',
        "# a comment mentioning shutil.copytree",
        "copytree = 1",
    ],
)
def test_prose_is_not_a_call(source: str, tmp_path: Path) -> None:
    """The rationale above names `copytree` repeatedly; it is not a violation."""
    module = tmp_path / "sample.py"
    module.write_text(source, encoding="utf-8")
    assert not _calls(module)
