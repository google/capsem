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

`REMAINING_SHELL_RECIPES` is a ratchet, not an exemption list. A recipe may
leave it; nothing may join it; and a recipe that has already been extracted
must be struck from it, so the list cannot quietly describe a past that is no
longer true.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import tomllib
from pathlib import Path

import pytest


PROJECT_ROOT = Path(__file__).resolve().parents[1]
GATE_PACKAGE = PROJECT_ROOT / "src" / "capsem" / "gate"

LIMITS = tomllib.loads((PROJECT_ROOT / "pyproject.toml").read_text(encoding="utf-8"))[
    "tool"
]["capsem"]["gate"]

# Recipes whose bodies are still inline shell, pending extraction. Ordered by
# the size of the body, because that is the order they are worth doing in.
REMAINING_SHELL_RECIPES = frozenset(
    {
        "_test-candidate-run",
        "_gate-assets",
        "_cross-compile",
        "_prove-linux-deb",
        "_pack-initrd",
        "smoke",
        "_ensure-service",
        "_gate-linux-rust",
        "test",
        "_install-tools",
        "_gate-host-package-sbom",
        "_build-host-image",
        "_check-assets",
        "_build-assets",
        "logs",
        "_build-image-template",
        "release-binaries",
        "_build-ui",
        "_build-kernel",
        "_build-rootfs",
        "_sign-release",
        "dev",
        "release-profile",
        "_ensure-dev-ready",
        "_sign",
        "_test-functional",
        "doctor",
        "_dev-ui",
        "exec",
        "shell",
        "_check-generated-settings",
        "_materialize-config",
        "_generate-settings",
    }
)

# A body line that opens a shell construct is inline logic whether or not the
# recipe declared a shebang: `just` hands each line to a shell, so a `for` loop
# spread over continuations is a program with no test around it. `_pnpm-install`
# is exactly that, and its failure inside a job with no pnpm is one of the
# defects that started this.
SHELL_CONTROL_FLOW = ("if ", "for ", "while ", "case ", "until ", "trap ")

RECIPES_WITH_INLINE_CONTROL_FLOW = frozenset({"_pnpm-install"})


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


def test_no_new_recipe_grows_a_shell_body() -> None:
    recipes = _recipes()
    assert recipes, "no recipes parsed; this guard would pass vacuously"

    inline = {name for name, recipe in recipes.items() if recipe["shebang"]}

    assert not inline - REMAINING_SHELL_RECIPES, (
        "these recipes have inline shell bodies that no test can reach; put "
        "the logic in src/capsem/gate/ and call it from a one-line recipe: "
        f"{sorted(inline - REMAINING_SHELL_RECIPES)}"
    )


def test_the_extraction_ratchet_never_runs_backwards() -> None:
    """An extracted recipe must leave the list, or the list becomes fiction."""
    recipes = _recipes()
    inline = {name for name, recipe in recipes.items() if recipe["shebang"]}

    stale = sorted(REMAINING_SHELL_RECIPES - inline)
    assert not stale, (
        "these recipes no longer carry inline shell -- remove them from "
        f"REMAINING_SHELL_RECIPES so the remaining work stays honest: {stale}"
    )

    gone = sorted(REMAINING_SHELL_RECIPES - set(recipes))
    assert not gone, f"these recipes no longer exist: {gone}"


def test_a_dispatching_recipe_stays_short_enough_to_read() -> None:
    ceiling = LIMITS["max_recipe_lines"]
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
        if recipe["shebang"] or name in RECIPES_WITH_INLINE_CONTROL_FLOW:
            continue
        opening = [
            line
            for line in _executable_lines(recipe)
            if line.lstrip().startswith(SHELL_CONTROL_FLOW)
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
    ceiling = LIMITS["max_module_lines"]
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

    Every subcommand is contributed by the module that implements it, so this
    file has no reason to name a command, run one, or branch on what one means.
    """
    cli = (GATE_PACKAGE / "cli.py").read_text(encoding="utf-8")

    assert "add_parser(" not in cli, (
        "a subcommand defined here is a subcommand defined away from its "
        "implementation; add `register(subparsers)` to the owning module"
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
