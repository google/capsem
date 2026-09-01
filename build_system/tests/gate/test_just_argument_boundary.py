"""Every recipe argument crosses exactly one argv boundary.

`just` interpolates `{{value}}` into the recipe body as *source*, and the host
shell then parses it. Every validation Python performs happens after that, so
Python cannot contain this: by the time the gate sees an argument the shell has
already run whatever was in it.

    $ just --dry-run release-binaries 'nightly; echo HOST_PWNED'
    uv run --project build_system --frozen capsem-gate release-binaries nightly; echo HOST_PWNED

The recipes are discovered rather than listed. A hand-maintained list of five
public recipes is what let `build`, `build-all`, and every CI-facing asset
primitive stay unquoted while this file was green -- and the old check only
asked whether *any* quote appeared on the line, so `dev`'s quoted surface
masked the raw `{{ARGS}}` after it.

What is asserted is the argv, not the presence of a quote: the payload has to
come back as one exact token, which is the only phrasing that cannot be
satisfied by quoting the wrong half of the line.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]

pytestmark = pytest.mark.skipif(shutil.which("just") is None, reason="just is not installed")

#: One payload carrying every metacharacter class that matters, and a space,
#: so a single rendering answers "is this data" for all of them at once.
HOSTILE = "a b; echo HOST_PWNED | tee $(echo sub) `id` && :"

#: Each on its own, so a failure names which class escaped.
CLASSES = (
    "a; echo HOST_PWNED",
    "a | echo HOST_PWNED",
    "a > /tmp/capsem-boundary-probe-HOST_PWNED",
    "a $(echo HOST_PWNED)",
    "a `echo HOST_PWNED`",
    "a && echo HOST_PWNED",
    "a 'HOST_PWNED'",
    'a "HOST_PWNED"',
    "--HOST_PWNED",
)

MARKER = "HOST_PWNED"


def _dump() -> dict:
    result = subprocess.run(
        ["just", "--dump", "--dump-format", "json"],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
        check=True,
    )
    return json.loads(result.stdout)


RECIPES = _dump()["recipes"]


def _parameters(kinds: set[str]) -> list[tuple[str, tuple[str, ...]]]:
    """Every recipe whose parameters are all of the given kinds."""
    found = []
    for name, recipe in sorted(RECIPES.items()):
        parameters = recipe.get("parameters") or []
        if parameters and {p["kind"] for p in parameters} <= kinds:
            found.append((name, tuple(p["name"] for p in parameters)))
    return found


SINGULAR = _parameters({"singular"})
VARIADIC = _parameters({"star", "plus"})
JOINED_VARIADIC = [
    item for item in VARIADIC if "positional-arguments" not in RECIPES[item[0]]["attributes"]
]


def _rendered(recipe: str, arguments: list[str], cwd: Path | None = None) -> str:
    result = subprocess.run(
        ["just", "--dry-run", recipe, *arguments],
        cwd=cwd or PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )
    # `--dry-run` prints the recipe body it *would* run, to stderr.
    return result.stderr + result.stdout


#: Every program a recipe body can reach, shadowed by a receiver that prints
#: its argv. `shlex` was not enough: it treats `"a $(echo x)"` as one token,
#: while a real shell expands the substitution inside the double quotes. That
#: is exactly the difference between `quote(profile)` and `"{{profile}}"`, and
#: a test that cannot see it blesses the second.
SHADOWED = (
    "uv",
    "just",
    "cargo",
    "pnpm",
    "npm",
    "node",
    "docker",
    "git",
    "sh",
    "bash",
    "python3",
    "scripts",
)

RECEIVER = '#!/bin/sh\nfor argument in "$@"; do printf \'ARG:%s\\n\' "$argument"; done\n'


@pytest.fixture(scope="module")
def receiver(tmp_path_factory: pytest.TempPathFactory) -> Path:
    """A PATH where every program prints what it was actually handed."""
    directory = tmp_path_factory.mktemp("receiver")
    for name in SHADOWED:
        program = directory / name
        program.write_text(RECEIVER, encoding="utf-8")
        program.chmod(0o755)
    return directory


def _argv(line: str, receiver: Path) -> list[str]:
    """What a real shell would pass, running this line for real.

    The programs are shadowed, so nothing the recipe would have done happens;
    what survives is the argument vector, which is the whole question.
    """
    result = subprocess.run(
        ["/bin/sh", "-c", line],
        capture_output=True,
        text=True,
        timeout=60,
        env={"PATH": str(receiver), "HOME": str(receiver)},
        cwd=receiver,
    )
    return [
        entry.removeprefix("ARG:")
        for entry in result.stdout.splitlines()
        if entry.startswith("ARG:")
    ]


def _assert_one_argument(rendered: str, payload: str, context: str, receiver: Path) -> None:
    """Wherever the payload reached the shell, it arrived as one argument."""
    seen = False
    for line in rendered.splitlines():
        if MARKER not in line:
            continue
        seen = True
        arguments = _argv(line.strip(), receiver)
        assert payload in arguments, (
            f"{context} does not hand the payload over as one argument:\n"
            f"  rendered: {line.strip()}\n"
            f"  received: {arguments}\n"
            "Interpolate it through `{{quote(...)}}`; double quotes are not "
            "equivalent, since `$(...)` and backticks still expand inside them."
        )
    assert seen or MARKER not in rendered


def test_the_recipe_inventory_is_discovered_and_not_empty() -> None:
    """The guard this file is: nothing here is a hand-maintained list.

    Five recipes were enumerated by hand, and the eight that were not are
    exactly the ones that were unquoted.
    """
    assert len(SINGULAR) >= 10, SINGULAR
    covered = {name for name, _ in SINGULAR} | {name for name, _ in VARIADIC}
    declared = {name for name, recipe in RECIPES.items() if recipe.get("parameters")}
    assert covered == declared, f"parameterized recipes nothing checks: {declared - covered}"


@pytest.mark.parametrize(("recipe", "parameters"), SINGULAR, ids=[n for n, _ in SINGULAR])
def test_every_singular_parameter_stays_one_argument(
    recipe: str, parameters: tuple[str, ...], receiver: Path
) -> None:
    """Public and private alike -- CI calls the private ones with matrix values."""
    _assert_one_argument(
        _rendered(recipe, [HOSTILE] * len(parameters)), HOSTILE, f"`just {recipe}`", receiver
    )


@pytest.mark.parametrize("payload", CLASSES)
def test_every_metacharacter_class_survives_as_data(payload: str, receiver: Path) -> None:
    for recipe, parameters in SINGULAR:
        _assert_one_argument(
            _rendered(recipe, [payload] * len(parameters)),
            payload,
            f"`just {recipe}`",
            receiver,
        )


@pytest.mark.parametrize(
    ("recipe", "parameters"), JOINED_VARIADIC, ids=[n for n, _ in JOINED_VARIADIC]
)
def test_a_variadic_recipe_hands_over_one_joined_argument(
    recipe: str, parameters: tuple[str, ...], receiver: Path
) -> None:
    """`just` joins a variadic with spaces before interpolating it.

    Exact per-element boundaries therefore cannot survive, so the only honest
    contract is the one `exec` documents: the whole thing is a single string
    handed to one parameter. What must not happen is that string becoming
    shell source, which is what an unquoted `{{ARGS}}` does.
    """
    parts = ["first arg", HOSTILE]
    _assert_one_argument(_rendered(recipe, parts), " ".join(parts), f"`just {recipe}`", receiver)


def test_cache_preserves_each_variadic_argument(receiver: Path, tmp_path: Path) -> None:
    """The cache CLI accepts arbitrary options, including multiword values."""
    path = tmp_path / "path"
    path.mkdir()
    path.joinpath("uv").symlink_to(receiver / "uv")
    result = subprocess.run(
        [shutil.which("just") or "just", "cache", "prune", "--reason", HOSTILE],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
        env={"PATH": f"{path}:{os.environ['PATH']}", "HOME": str(receiver)},
        check=True,
    )
    arguments = [
        entry.removeprefix("ARG:")
        for entry in result.stdout.splitlines()
        if entry.startswith("ARG:")
    ]
    assert arguments[-4:] == ["dispatch", "prune", "--reason", HOSTILE]


def test_no_recipe_takes_an_unquotable_variadic_passthrough() -> None:
    """A variadic interpolated into source has no safe spelling.

    `quote()` collapses it to one argument and source quoting cannot preserve
    the original boundaries. A true passthrough uses the recipe's
    `positional-arguments` attribute and shell `"$@"`, as `cache` does.
    """
    body = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")

    for variable in ("ARGS", "CMD"):
        assert f"{{{{{variable}}}}}" not in body, (
            f"{variable} is interpolated raw; `just` cannot preserve argument "
            "boundaries through a variadic, so this has no safe spelling"
        )


def test_the_checkout_path_survives_a_directory_name_with_spaces(
    tmp_path: Path, receiver: Path
) -> None:
    """`justfile_directory()` is data too, and it is not always tame.

    Nothing renders it hostile, but a checkout under `~/My Projects/` is
    ordinary and an unquoted interpolation splits it into two arguments.
    """
    workspace = tmp_path / "a directory with spaces"
    workspace.mkdir()
    shutil.copy(PROJECT_ROOT / "justfile", workspace / "justfile")

    rendered = _rendered("_bootstrap", [], cwd=workspace)

    for line in rendered.splitlines():
        if "bootstrap.sh" not in line:
            continue
        assert str(workspace / "bootstrap.sh") in _argv(line.strip(), receiver), (
            f"the checkout path is split by an unquoted interpolation: {line.strip()}"
        )


def test_a_recipe_name_is_never_built_from_an_argument() -> None:
    """`just _dev-{{surface}}` makes the *recipe* attacker-chosen.

    Quoting cannot fix this one: the value is not an argument, it is part of
    the name being dispatched. The set of development surfaces is finite and
    known, so it is selected rather than constructed.
    """
    body = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")

    assert "just _dev-{{surface}}" not in body, (
        "the dev surface is interpolated into a recipe name; dispatch the "
        "known set instead of constructing the name from input"
    )
