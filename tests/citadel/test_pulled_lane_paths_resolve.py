"""Citadel guard: a test may not resolve a `target/` tree a pulled lane lacks.

A release lane qualifies from a prefix carrying only tracked files, so anything
under `target/` is absent unless the prefix links it or a step builds it. The
checked-in tests resolve `PROJECT_ROOT / "target" / ...` in about forty places
and are right to -- a test should not have to know whether this run built its
inputs or was handed them.

Three binary-release dispatches were spent on exactly that mismatch, one
directory at a time: `target/debug` for the host binaries, then `target/config`
for the materialized profiles, with `--maxfail=5` hiding whatever stood behind
them. Each fix was applied at the site that failed rather than to the class.

The class is checkable without running anything: for each `target/` subtree a
test resolves, either the prefix links it for a pulled lane, or a step in that
lane builds it, or the test skips when it is absent. Anything else is a
dispatch waiting to happen.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
#: Both first-party trees that run inside a lane. Scripts were the hole the
#: first version left: `integration_test.py` resolves `target/config` and
#: `mock_server.py` resolves `target/debug`, exactly like the tests do, and
#: nothing was checking them.
SOURCES = (PROJECT_ROOT / "tests", PROJECT_ROOT / "scripts")

#: What `cargotarget.link_prefix_trees` points at staged input for a pulled
#: lane. Adding one here means teaching the prefix to link it, not just listing
#: it: the guard exists to keep those two facts together.
LINKED_FOR_A_PULLED_LANE = {"debug", "config"}

#: Trees a pulled lane genuinely does not have and does not need, each because
#: something already decides so out loud.
#: Trees a run *creates*. A script naming its own output is not asking a lane
#: to provide anything, which is the whole distinction this guard turns on.
CREATED_BY_THE_RUN = {
    "ironbank-assets",
    "local-release-glowup",
    "macos-package-boot",
    "macos-release-glowup",
    "macos-tart-glowup",
    "release",
    "storage",
    "tart-readiness",
}

DECLARED_ABSENT = {
    # `conftest._required_artifacts_for_run` drops this for a release lane: it
    # is a source-build intermediate, and requiring it there would force a
    # rebuild that proves nothing about the pulled package.
    "linux-agent",
}

#: Read through the AST rather than by pattern. The first version matched the
#: literal `PROJECT_ROOT`, and thirty-three modules spell the same thing `ROOT`
#: -- so a test resolving `ROOT / "target/..."` walked straight past a guard
#: written to stop exactly that.
_TARGET = "target"


def _first_party() -> list[Path]:
    """Every checked-in Python file that runs inside a lane."""
    return sorted(path for source in SOURCES for path in source.rglob("*.py"))


def _checkout_roots(tree: ast.Module) -> set[str]:
    """Module-level names bound to this checkout, however they are spelled.

    A name is a checkout root when it is assigned from `Path(__file__)`, which
    is how every one of them is built. A `tmp_path` fixture is a parameter and
    never appears here, which is the distinction that matters: those trees are
    created by the test and are supposed to be absent until it makes them.
    """
    roots: set[str] = set()
    for node in tree.body:
        if not isinstance(node, ast.Assign) or not isinstance(node.value, ast.expr):
            continue
        rendered = ast.dump(node.value)
        if "__file__" not in rendered:
            continue
        for target in node.targets:
            if isinstance(target, ast.Name):
                roots.add(target.id)
    return roots


def _joined(node: ast.expr) -> list[str]:
    """Flatten `a / b / c` into its literal string parts, root name first."""
    parts: list[str] = []
    while isinstance(node, ast.BinOp) and isinstance(node.op, ast.Div):
        if isinstance(node.right, ast.Constant) and isinstance(node.right.value, str):
            parts.insert(0, node.right.value)
        else:
            parts.insert(0, "")
        node = node.left
    if isinstance(node, ast.Name):
        parts.insert(0, node.id)
    return parts


def _resolved_trees() -> dict[str, list[str]]:
    """Every `target/<tree>` a test resolves from its checkout root."""
    trees: dict[str, list[str]] = {}
    for path in _first_party():
        tree = ast.parse(path.read_text(encoding="utf-8"))
        roots = _checkout_roots(tree)
        if not roots:
            continue
        for node in ast.walk(tree):
            if not isinstance(node, ast.BinOp) or not isinstance(node.op, ast.Div):
                continue
            parts = _joined(node)
            if len(parts) < 2 or parts[0] not in roots:
                continue
            segments = [segment for part in parts[1:] for segment in part.split("/") if segment]
            if len(segments) < 2 or segments[0] != _TARGET:
                continue
            tree_name = segments[1]
            # A name with a suffix is a file, and every one found this way is
            # scratch the test writes itself -- a run-state json, a build lock,
            # a fixture blob. What a lane must provide is a directory of
            # artifacts, and none of those carry an extension.
            if "." in tree_name:
                continue
            trees.setdefault(tree_name, []).append(str(path.relative_to(PROJECT_ROOT)))
    return trees


def test_the_guard_has_subjects() -> None:
    """A rule over nothing asserts nothing."""
    resolved = _resolved_trees()
    assert resolved, "no test resolves a target/ tree; the pattern has drifted"
    assert "debug" in resolved, "target/debug is resolved by tests; the pattern missed it"


def test_every_resolved_target_tree_exists_in_a_pulled_lane() -> None:
    """Either the prefix links it, or something says out loud that it is absent.

    A new entry here is not a licence to add it to the allow list. It means
    deciding which: teach `cargotarget.link_prefix_trees` to link the tree, or
    record why a pulled lane does not need it and make the tests that read it
    skip when it is missing.
    """
    unaccounted = {
        name: sorted(set(readers))
        for name, readers in _resolved_trees().items()
        if name not in LINKED_FOR_A_PULLED_LANE
        and name not in DECLARED_ABSENT
        and name not in CREATED_BY_THE_RUN
    }
    # Trees a test creates under tmp_path or names only to prove absence are
    # not resolved from a real checkout, so they carry their own marker.
    unaccounted = {
        name: readers
        for name, readers in unaccounted.items()
        if not name.startswith(("synthetic", "missing"))
    }
    assert not unaccounted, (
        "these resolve a target/ tree that a release lane's prefix does not "
        f"have: {unaccounted}. Link it in cargotarget.link_prefix_trees, or "
        "declare it absent and skip when it is missing."
    )


@pytest.mark.parametrize("tree", sorted(LINKED_FOR_A_PULLED_LANE))
def test_the_prefix_actually_links_what_this_guard_claims(tree: str) -> None:
    """The allow list must describe the code, not replace it."""
    source = (PROJECT_ROOT / "build_system" / "builder" / "gate" / "cargotarget.py").read_text(
        encoding="utf-8"
    )
    linked = ast.parse(source)
    literals = {
        node.value
        for node in ast.walk(linked)
        if isinstance(node, ast.Constant) and isinstance(node.value, str)
    }
    assert tree in literals or f"{tree}" in source, (
        f"this guard claims a pulled lane links target/{tree}, but "
        "cargotarget never names it"
    )
