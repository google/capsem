"""Release-site builds go through `helpers.release_site`, never by hand.

Astro stages prerendered chunks at a path fixed under the project root, so two
concurrent builds of `release-site` delete each other's staging mid-prerender
and no `--outDir` makes them independent. Every build on a host must therefore
serialize, and -- because the pages land in the one shared `release-site/dist`
-- every reader must take its own copy before the next build overwrites them.

Both rules had been rediscovered by hand seven times across thirteen files.
Four of those copies took a lock that covered the build but not the reads it
existed to protect, which is why the release-site gates failed on a different
assertion every run under `pytest -n`.

**Serialize.** A module that spawns a release-site build reaches the lock
through `helpers.release_site` -- `build_release_site` for a rendered graph,
`release_site_build_lock` for a `build:channel` that manages its own output.

**Snapshot.** `release-site/dist` is the shared staging area, not a place to
read results from. Only the helper may name it; callers read the private
directory a build returns.

A wrapper nobody is obliged to use is a suggestion. These make it the only way.
"""

from __future__ import annotations

import ast
import re
from pathlib import Path

from capsem.gate.shelllex import tokenize
from capsem.gate.shellnodes import Function, arm_named, commands, suppressed, walk
from capsem.gate.shellparse import parse

PROJECT_ROOT = Path(__file__).resolve().parent.parent
TESTS_ROOT = PROJECT_ROOT / "tests"

# Owns both rules, so it is the one place allowed to implement them.
OWNER = TESTS_ROOT / "helpers" / "release_site.py"
SHELL_OWNER = PROJECT_ROOT / "scripts" / "check-web-surface.sh"
BUILD_SURFACES = ("frontend-build", "docs", "site", "release-site-build")

# `["pnpm", ...]` as a subprocess argv. Deliberately not a bare "pnpm" match:
# several gates assert on the *text* of shell scripts that run pnpm, and
# asserting on a command is not spawning one.
PNPM_ARGV = re.compile(r'\[\s*"pnpm"')

# A direct install mutates the same release-site/node_modules tree every Astro
# build reads. The shared helper owns that transaction with the build lock.
PNPM_INSTALL_ARGV = re.compile(r'\[\s*"pnpm"[^\]]*"install"', re.DOTALL)

# Builds `release-site` either by naming it in the argv or by running there.
RELEASE_SITE_BUILD = re.compile(r'"--dir",\s*"release-site"|cwd=\w+\s*/\s*"release-site"')

# The host `release-site/dist`, built as a path. Container-side literals such
# as "/src/release-site/dist" name a bind mount inside the install image and
# are a different thing entirely.
HOST_DIST = re.compile(r'"release-site"\s*(?:/|,)\s*"dist"')

REACHES_HELPER = re.compile(r"from helpers\.release_site import|helpers\.release_site")


def _test_sources() -> list[Path]:
    return sorted(path for path in TESTS_ROOT.rglob("*.py") if path != OWNER)


def test_release_site_builds_take_the_shared_lock() -> None:
    offenders = []
    for path in _test_sources():
        source = path.read_text(encoding="utf-8")
        if not (PNPM_ARGV.search(source) and RELEASE_SITE_BUILD.search(source)):
            continue
        if not REACHES_HELPER.search(source):
            offenders.append(path.relative_to(PROJECT_ROOT))

    assert not offenders, (
        "these modules spawn a release-site build without reaching the shared "
        "lock in helpers.release_site, so they race every other build on the "
        f"host: {[str(path) for path in offenders]}"
    )


def test_only_the_helper_installs_release_site_dependencies() -> None:
    offenders = []
    for path in _test_sources():
        source = path.read_text(encoding="utf-8")
        if PNPM_INSTALL_ARGV.search(source) and RELEASE_SITE_BUILD.search(source):
            offenders.append(path.relative_to(PROJECT_ROOT))

    assert not offenders, (
        "these modules mutate release-site/node_modules outside the shared "
        "workspace lock; move install and build into helpers.release_site: "
        f"{[str(path) for path in offenders]}"
    )


def test_only_the_helper_names_the_shared_dist() -> None:
    offenders = [
        path.relative_to(PROJECT_ROOT)
        for path in _test_sources()
        if HOST_DIST.search(path.read_text(encoding="utf-8"))
    ]

    assert not offenders, (
        "release-site/dist is shared staging that the next build overwrites; "
        "read the private directory build_release_site returns instead of "
        f"naming it: {[str(path) for path in offenders]}"
    )


def _unlocked_shell_surfaces(source: str) -> list[str]:
    tree = parse(source)
    return [
        surface
        for surface in BUILD_SURFACES
        if not any(
            command.program == "astro_build" and "pnpm" in command.argv
            for command in commands(arm_named(tree, surface) or [])
        )
    ]


def _astro_function(source: str) -> Function | None:
    return next(
        (
            node
            for node in walk(parse(source))
            if isinstance(node, Function) and node.name == "astro_build"
        ),
        None,
    )


def _python_lock_parts(source: str) -> tuple[str, ...]:
    def strings(node: ast.expr) -> tuple[str, ...]:
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Div):
            return strings(node.left) + strings(node.right)
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            return (node.value,)
        if isinstance(node, ast.Name) and node.id == "PROJECT_ROOT":
            return ()
        return ()

    tree = ast.parse(source)
    for node in tree.body:
        if not isinstance(node, ast.Assign):
            continue
        if any(
            isinstance(target, ast.Name) and target.id == "_LOCK_PATH" for target in node.targets
        ):
            return strings(node.value)
    return ()


def _shell_lock_parts(source: str) -> tuple[str, ...]:
    for token in tokenize(source):
        if token.value.startswith("BUILD_LOCK=$ROOT/"):
            return tuple(token.value.removeprefix("BUILD_LOCK=$ROOT/").split("/"))
    return ()


def test_every_shell_astro_build_takes_the_cross_process_lock() -> None:
    source = SHELL_OWNER.read_text(encoding="utf-8")
    assert not _unlocked_shell_surfaces(source)
    assert 'source "$ROOT/scripts/lib/exec_lock.sh"' in source
    function = _astro_function(source)
    assert function is not None
    assert [command.argv for command in commands(function.body)] == [
        ("run_with_exec_lock", "$BUILD_LOCK", "$@"),
    ]
    assert not suppressed(function.body)


def test_shell_and_python_builders_lock_the_same_repository_path() -> None:
    shell = _shell_lock_parts(SHELL_OWNER.read_text(encoding="utf-8"))
    python = _python_lock_parts(OWNER.read_text(encoding="utf-8"))
    assert shell == python == ("target", "capsem-release-site-build.lock")


def test_the_shell_lock_guard_rejects_an_unwrapped_build() -> None:
    source = SHELL_OWNER.read_text(encoding="utf-8")
    mutated = source.replace(
        "astro_build pnpm --dir docs run build",
        "pnpm --dir docs run build",
        1,
    )
    assert _unlocked_shell_surfaces(mutated) == ["docs"]


def test_the_shell_lock_guard_rejects_discarded_enforcement() -> None:
    source = SHELL_OWNER.read_text(encoding="utf-8")
    mutated = source.replace(
        'run_with_exec_lock "$BUILD_LOCK" "$@"',
        'run_with_exec_lock "$BUILD_LOCK" "$@" || true',
        1,
    )
    function = _astro_function(mutated)
    assert function is not None
    assert [command.program for command in suppressed(function.body)] == ["run_with_exec_lock"]
