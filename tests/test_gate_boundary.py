"""The justfile dispatches; `capsem.gate` decides. Both halves are held here.

The justfile reached 2457 lines, of which roughly 2070 were `bash` inside
recipe bodies. Nothing in that shell could be unit tested, so every defect in
it was found by running the forty-minute gate and reading the wreckage: an
installer handed a manifest URL before anything wrote the manifest, a release
version built from `$(date +%s)`, a log stream opened by a name that daily
rotation had already moved off, and an asset compatibility floor hardcoded
above the binary that shipped beside it.

Moving that logic into Python is only half the fix. The other half is making
the old shape unavailable, in both directions:

  the justfile        may not grow a shell body back
  `capsem.gate`       may not become one 2000-line file in a new language

`remaining_shell_recipes` in `config/gate.toml` is a ratchet, not an
exemption list. A recipe may
leave it; nothing may join it; and a recipe that has already been extracted
must be struck from it, so the list cannot quietly describe a past that is no
longer true.
"""

from __future__ import annotations

import ast
import json
import shutil
import subprocess
from pathlib import Path

import pytest

from capsem.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[1]
GATE_PACKAGE = PROJECT_ROOT / "src" / "capsem" / "gate"

CONFIG = gate_config.load(PROJECT_ROOT)
BOUNDARY = CONFIG.boundary


def _recipes() -> dict[str, dict]:
    """Every recipe, as `just` itself parses it.

    Parsed by `just`, not by a regex over the file: a guard that reimplements
    the parser eventually disagrees with it, and then it is guarding its own
    idea of the justfile.
    """
    just = shutil.which("just")
    assert just is not None, (
        "this contract reads the justfile through `just --dump`; the job "
        "running it must provision just (see the CI provisioning contract)"
    )
    dumped = subprocess.run(
        [just, "--dump", "--dump-format", "json"],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(dumped.stdout)["recipes"]


def _executable_lines(recipe: dict) -> list[str]:
    lines = ["".join(part for part in line if isinstance(part, str)) for line in recipe["body"]]
    return [line for line in lines if line.strip() and not line.strip().startswith("#")]


# ---------------------------------------------------------------------------
# The justfile side
# ---------------------------------------------------------------------------


def test_no_recipe_has_a_shell_body() -> None:
    """Not "fewer than before" -- none.

    The ratchet that tracked the outstanding extraction is gone, because the
    extraction is finished. An empty ratchet should be deleted rather than
    kept: a list describing no remaining work is a list that will be read as
    permission for some.
    """
    recipes = _recipes()
    assert recipes, "no recipes parsed; this guard would pass vacuously"

    inline = sorted(name for name, recipe in recipes.items() if recipe["shebang"])

    assert not inline, (
        "these recipes carry inline shell that no test can reach; put the "
        f"logic in src/capsem/gate/ and dispatch to it: {inline}"
    )


def test_a_dispatching_recipe_stays_short_enough_to_read() -> None:
    ceiling = BOUNDARY.max_recipe_lines
    oversized = {
        name: len(_executable_lines(recipe))
        for name, recipe in _recipes().items()
        if not recipe["shebang"] and len(_executable_lines(recipe)) > ceiling
    }

    assert not oversized, (
        f"a recipe body over {ceiling} executable lines is a program; move it "
        f"into src/capsem/gate/: {oversized}"
    )


def test_no_recipe_hides_shell_logic_without_a_shebang() -> None:
    """A `for` loop across continuation lines is still an untested program."""
    offenders = {}
    for name, recipe in _recipes().items():
        if recipe["shebang"] or name in BOUNDARY.recipes_with_inline_control_flow:
            continue
        opening = [
            line
            for line in _executable_lines(recipe)
            if line.lstrip().startswith(tuple(BOUNDARY.shell_control_flow))
        ]
        if opening:
            offenders[name] = opening

    assert not offenders, (
        "shell control flow in a recipe body is logic no test can reach: "
        f"{offenders}"
    )


# ---------------------------------------------------------------------------
# The package side
# ---------------------------------------------------------------------------


def test_no_gate_module_grows_into_the_justfile_it_replaced() -> None:
    ceiling = BOUNDARY.max_module_lines
    modules = sorted(GATE_PACKAGE.rglob("*.py"))
    assert len(modules) > 3, "scanned too few modules to trust this guard"

    oversized = {
        module.relative_to(PROJECT_ROOT).as_posix(): len(
            module.read_text(encoding="utf-8").splitlines()
        )
        for module in modules
        if len(module.read_text(encoding="utf-8").splitlines()) > ceiling
    }

    assert not oversized, (
        f"a gate module over {ceiling} lines is the 2000-line justfile growing "
        f"back in Python; split it by responsibility: {oversized}"
    )


def test_the_cli_only_parses_and_dispatches() -> None:
    """Business logic in the entry point is how one file becomes all of them.

    Every subcommand is contributed by the module that implements it, by
    subclassing `GateCommand`, so this file builds its parsers by looping over
    the registry and has no reason to name a command, run one, or branch on
    what one means.
    """
    from capsem.gate.command import GateCommand

    cli = (GATE_PACKAGE / "cli.py").read_text(encoding="utf-8")

    named = sorted(name for name in GateCommand.registry if f'"{name}"' in cli)
    assert not named, (
        f"the CLI names these commands: {named}. A subcommand spelled here is "
        "a subcommand defined away from its implementation; it should arrive "
        "through the registry."
    )
    assert "subprocess" not in cli, "the CLI dispatches; the modules run things"
    for smell in ("docker ", "cargo ", "pnpm ", "uv run"):
        assert smell not in cli, f"the CLI should not know about {smell.strip()!r}"


@pytest.mark.parametrize(
    "module", sorted(p.name for p in GATE_PACKAGE.glob("*.py"))
)
def test_every_gate_module_imports_on_its_own(module: str) -> None:
    """Independently importable, so it can be independently unit tested."""
    name = f"capsem.gate.{module.removesuffix('.py')}"
    if module in {"__init__.py", "__main__.py"}:
        pytest.skip("package entry points, exercised through the CLI")

    subprocess.run(
        ["python3", "-c", f"import {name}"],
        cwd=PROJECT_ROOT / "src",
        check=True,
        capture_output=True,
    )


# ---------------------------------------------------------------------------
# The type-checking ratchet
# ---------------------------------------------------------------------------


def test_the_strict_python_tree_needs_no_rules_held_back() -> None:
    """`src/` is checked with nothing disabled, and must stay that way.

    `ty` ran on `src/capsem` alone for a long time, so `scripts/` -- release
    machinery, not scratch -- had no type gate at all. Widening it meant
    holding some rules back on the trees that were never checked; holding any
    back on `src/` would give that ground away again.
    """
    settings = CONFIG.lint

    assert set(settings.strict_roots) <= set(settings.python_roots)
    assert "src" in settings.strict_roots

    strict = subprocess.run(
        ["uv", "run", "ty", "check", *settings.ty_flags, *settings.strict_roots],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )
    assert strict.returncode == 0, strict.stdout + strict.stderr


def test_the_type_ratchet_only_shrinks() -> None:
    """A rule that no longer fires must leave the list.

    Otherwise the ratchet stops describing outstanding work and starts
    describing policy, which is how a temporary exemption becomes permanent.
    """
    settings = CONFIG.lint
    held_back = list(settings.ty_ratchet)
    assert held_back, "an empty ratchet means the list should be deleted, not kept"

    relaxed = list(settings.relaxed_roots)
    still_firing = set()
    for rule in held_back:
        others = [
            flag
            for other in held_back
            if other != rule
            for flag in ("--ignore", other)
        ]
        result = subprocess.run(
            ["uv", "run", "ty", "check", *settings.ty_flags, *relaxed, *others],
            cwd=PROJECT_ROOT,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            still_firing.add(rule)

    assert set(held_back) == still_firing, (
        "these rules are held back but no longer fire; remove them from "
        f"config/gate.toml's ty_ratchet: {sorted(set(held_back) - still_firing)}"
    )


# ---------------------------------------------------------------------------
# What is still opaque, and why
# ---------------------------------------------------------------------------


def _call_sites() -> dict[str, list[ast.Call]]:
    """Every production `Call(...)`, by the module that builds it."""
    found: dict[str, list[ast.Call]] = {}
    for module in sorted((PROJECT_ROOT / "src/capsem/gate").glob("*.py")):
        if module.name == "actions.py":
            continue  # where `Call` is defined, not where one is built
        tree = ast.parse(module.read_text(encoding="utf-8"))
        sites = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Name)
            and node.func.id == "Call"
        ]
        if sites:
            found[module.name] = sites
    return found


def test_every_opaque_call_says_why_it_is_one() -> None:
    """The extraction ratchet is gone because it was empty, and a raw count
    would only invite cosmetic movement. What replaces it is a required answer.

    `Call` renders as prose, so a plan built from these says far less than one
    built from `Run` and `Copy`. Twenty of them shared a single rationale --
    "a package build carries signing material" -- which is true of exactly one.
    A reason that covers everything is not a reason.
    """
    undeclared = {
        name: len([site for site in sites if not any(k.arg == "why" for k in site.keywords)])
        for name, sites in _call_sites().items()
    }
    offenders = {name: count for name, count in undeclared.items() if count}

    assert not offenders, (
        f"these build a `Call` without saying why it cannot be a declared "
        f"action: {offenders}. Pass `why=Why.SECRETS`, `Why.DYNAMIC` or "
        f"`Why.COMPUTATION`."
    )


def test_only_the_package_build_claims_to_carry_secrets() -> None:
    """`Why.SECRETS` is the one reason a dry run genuinely must not render.

    Spreading it would re-create the old docstring's claim in a form the type
    system endorses, and make the real one unfindable.
    """
    claimed = sorted(
        module.name
        for module in (PROJECT_ROOT / "src/capsem/gate").glob("*.py")
        if module.name != "actions.py"
        and "Why.SECRETS" in module.read_text(encoding="utf-8")
    )

    assert claimed == ["crosscompile.py"], (
        "only the package build's environment holds the Tauri signing key; "
        f"these also claim to: {claimed}"
    )
