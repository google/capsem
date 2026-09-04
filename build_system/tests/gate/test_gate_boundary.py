"""The justfile dispatches; `capsem_builder.gate` decides. Both halves are held here.

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
  `capsem_builder.gate`       may not become one 2000-line file in a new language

`remaining_shell_recipes` in `config/gate.toml` is a ratchet, not an
exemption list. A recipe may
leave it; nothing may join it; and a recipe that has already been extracted
must be struck from it, so the list cannot quietly describe a past that is no
longer true.
"""

from __future__ import annotations

import ast
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

import embedded_shell
import pytest
from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[3]
GATE_PACKAGE = PROJECT_ROOT / "build_system" / "builder" / "gate"

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
        f"logic in build_system/builder/gate/ and dispatch to it: {inline}"
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
        f"into build_system/builder/gate/: {oversized}"
    )


#: `just` substitutes `{{ ... }}` before a shell sees the line, so it is not
#: shell and must not be read as any. `just doctor` is written
#: `{{ if fix == "fix" { "--fix" } else { "" } }}`, which a keyword search
#: reads as an `if` and flags on the approved public surface.
_JUST_INTERPOLATION = re.compile(r"\{\{.*?\}\}", re.S)


def _control_flow_in(lines: list[str]) -> list[str]:
    """Recipe lines that open shell control flow, wherever it sits.

    This was `line.lstrip().startswith(("if ", "for ", ...))`, which only ever
    saw column zero -- so

        test -f x && for f in *; do rm "$f"; done

    read as a `test`, and the rule that a recipe is a dispatch or a single
    command was enforced only against people who wrote their loop first on the
    line. It also could not tell a keyword from the same letters inside a
    string.

    Tokenising answers both: a command is a verb and its words, and `echo "if
    ..."` has one word.
    """
    keywords = {word.strip() for word in BOUNDARY.shell_control_flow}
    found = []
    for line in lines:
        shell = _JUST_INTERPOLATION.sub(" ", line)
        commands = embedded_shell.commands(shell)
        if any(command and command[0] in keywords for command in commands):
            found.append(line)
    return found


def test_no_recipe_hides_shell_logic_without_a_shebang() -> None:
    """A `for` loop across continuation lines is still an untested program."""
    offenders = {}
    for name, recipe in _recipes().items():
        if recipe["shebang"] or name in BOUNDARY.recipes_with_inline_control_flow:
            continue
        opening = _control_flow_in(_executable_lines(recipe))
        if opening:
            offenders[name] = opening

    assert not offenders, (
        f"shell control flow in a recipe body is logic no test can reach: {offenders}"
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
    from capsem_builder.gate.command import GateCommand

    cli = (GATE_PACKAGE / "cli.py").read_text(encoding="utf-8")

    named = sorted(name for name in GateCommand.registry if f'"{name}"' in cli)
    assert not named, (
        f"the CLI names these commands: {named}. A subcommand spelled here is "
        "a subcommand defined away from its implementation; it should arrive "
        "through the registry."
    )
    assert "subprocess" not in cli, "the CLI dispatches; the modules run things"
    for smell in ("docker ", "cargo ", "pnpm ", "uv run --project build_system --frozen"):
        assert smell not in cli, f"the CLI should not know about {smell.strip()!r}"


@pytest.mark.parametrize("module", sorted(p.name for p in GATE_PACKAGE.glob("*.py")))
def test_every_gate_module_imports_on_its_own(module: str) -> None:
    """Independently importable, so it can be independently unit tested."""
    name = f"capsem_builder.gate.{module.removesuffix('.py')}"
    if module in {"__init__.py", "__main__.py"}:
        pytest.skip("package entry points, exercised through the CLI")

    subprocess.run(
        [sys.executable, "-c", f"import {name}"],
        cwd=PROJECT_ROOT,
        check=True,
        capture_output=True,
    )


# ---------------------------------------------------------------------------
# The type-checking ratchet
# ---------------------------------------------------------------------------


def test_the_strict_python_tree_needs_no_rules_held_back() -> None:
    """The strict package is checked with nothing disabled and stays that way.

    Ty once ran on only the retired package while release machinery had no
    type gate at all. Widening it meant holding some rules back on the trees
    that were never checked; holding any back on the strict owner would give
    that ground away again.
    """
    settings = CONFIG.lint

    assert set(settings.strict_roots) <= set(settings.python_roots)
    assert "build_system/builder" in settings.strict_roots

    from capsem_builder.gate.sourcechecks import ty_argv

    strict = subprocess.run(
        ty_argv(CONFIG, settings.strict_roots),
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )
    assert strict.returncode == 0, strict.stdout + strict.stderr


# ---------------------------------------------------------------------------
# What is still opaque, and why
# ---------------------------------------------------------------------------


def _call_sites() -> dict[str, list[ast.Call]]:
    """Every production `Call(...)`, by the module that builds it."""
    found: dict[str, list[ast.Call]] = {}
    for module in sorted((PROJECT_ROOT / "build_system/builder/gate").glob("*.py")):
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


def test_every_opaque_call_declares_a_real_reason() -> None:
    """A closed kind, a reason somebody wrote, and a declared effect set.

    The minimum length is not quality by itself, which is why the placeholders
    are named: "temporary", "misc", "legacy" and "TODO" are how an exemption
    stops describing outstanding work and starts describing policy.
    """
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from capsem_builder.gate.actions import Call
    from helpers.gate import gate_plan

    placeholders = ("temporary", "misc", "legacy", "todo", "for now", "tbd")
    calls = [
        action
        for command in ("candidate", "assets", "install")
        for step in gate_plan(command).steps
        for action in step.actions
        if isinstance(action, Call)
    ]
    assert calls, "no opaque work found, so this guard is asking nothing"

    for call in calls:
        reason = call.justification.reason.lower()
        assert not any(word in reason for word in placeholders), (
            f"{call.render()} justifies itself with a placeholder: {reason!r}"
        )
        assert call.justification.effects or call.justification.kind.value == "pure-inspection"


def test_secret_bearing_work_is_only_the_package_build() -> None:
    """The one phase whose environment holds the Tauri key.

    Spreading the label would recreate the old docstring's claim in a form the
    type system endorses, and make the real one unfindable.
    """
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from capsem_builder.gate.actions import Call
    from capsem_builder.gate.opacity import OpaqueKind
    from helpers.gate import gate_plan

    secretive = sorted(
        step.label
        for step in gate_plan("candidate").steps
        for action in step.actions
        if isinstance(action, Call) and action.justification.kind is OpaqueKind.SECRET_BEARING
    )

    assert secretive == ["package.arm64.build", "package.x86_64.build"]


def test_closed_gate_vocabularies_refuse_raw_strings_at_runtime() -> None:
    """Untyped callers cannot bypass the enum-only constructor seams."""
    from typing import Any, cast

    from capsem_builder.gate.installimage import _step_label
    from capsem_builder.gate.opacity import machine_effects

    dynamic_effects = cast(Any, machine_effects)
    dynamic_label = cast(Any, _step_label)
    with pytest.raises(TypeError, match="Effect enum"):
        dynamic_effects("process")
    with pytest.raises(TypeError, match="InstallImageStep enum"):
        dynamic_label("install.cache")


def test_install_lifecycle_labels_flow_through_the_enum_converter() -> None:
    """The install steps in the plan are exactly the enum's members.

    Asked of the plan rather than of a module's source text. The previous
    version grepped `installimage.py` for `_step_label(InstallImageStep.X)`,
    which failed the moment the plan composition moved to `installplan.py` --
    a change that moved no step, renamed nothing and altered no order. A
    contract that breaks on where code lives is measuring the wrong thing.

    This states the property instead: every `install.` label the gate builds
    comes from the closed enum, and every member appears. A literal string
    label would show up as a plan label with no enum member behind it.
    """
    from capsem_builder.gate.installimage import InstallImageStep, _step_label
    from helpers.gate import gate_plan

    expected = {_step_label(member) for member in InstallImageStep}
    built = {label for label in gate_plan("candidate").labels if label.startswith("install.")}
    assert built == expected, (
        f"install steps in the plan {sorted(built)} are not the enum's "
        f"{sorted(expected)}; a label was spelled rather than derived"
    )


def test_control_flow_is_found_wherever_it_sits_on_the_line() -> None:
    """`startswith` only ever saw column zero.

    A recipe body is shell, and shell does not care where a keyword sits:

        test -f x && for f in *; do rm "$f"; done

    is a loop, and a prefix check reads it as a `test`. The rule this guard
    enforces -- a recipe is a dispatch or a single command -- was therefore
    enforced only against people who wrote their loop at the start of a line.
    """
    body = ['test -f x && for f in *; do rm "$f"; done']
    assert _control_flow_in(body) == body


def test_just_interpolation_is_not_shell() -> None:
    """`{{ if fix == "fix" { "--fix" } else { "" } }}` is `just`, not bash.

    It is substituted before a shell ever sees the line, and `just doctor`
    uses exactly this. A guard that reads it as an `if` would flag the
    approved surface.
    """
    assert (
        _control_flow_in(
            [
                '@build_system/scripts/doctor/doctor-common.sh {{ if fix == "fix" { "--fix" } else { "" } }}'
            ]
        )
        == []
    )


def test_a_keyword_inside_a_string_is_not_control_flow() -> None:
    """`echo "if you see this, the build worked"` is an echo."""
    assert _control_flow_in(['echo "if you see this, the build worked"']) == []


def test_a_leading_keyword_is_still_caught() -> None:
    """What the prefix check did catch, it must keep catching."""
    assert _control_flow_in(["for f in *; do rm $f; done"])
