"""Resolve what a Just recipe actually runs, following its dependency graph.

CI contracts need to answer "does this job end up running pnpm?" when the job
only says `just _cross-compile`. Answering it by substring lets a hardcoded
recipe name stand in for the real graph, which is how the pnpm cache-ownership
gate ended up accepting exactly one recipe by name.

Reachability deliberately over-approximates: it follows recipe dependencies and
`just <recipe>` calls without modelling shell branches, so a recipe whose
conditional path skips a command still counts as reaching it. For provisioning
questions that bias is the safe one -- a spare tool costs seconds, a missing one
is a red gate. Do not use it to assert that a command is *never* run.
"""

from __future__ import annotations

import functools
import os
import re
from pathlib import Path

from capsem_builder.gate.shellnodes import commands
from capsem_builder.gate.shellparse import parse as parse_shell

ROOT = Path(os.environ.get("CAPSEM_REPOSITORY_ROOT", Path.cwd())).resolve()

# A recipe header is `name params: deps`, anchored at column zero so recipe
# bodies (always indented) cannot match. `:(?![=])` keeps `foo := bar`
# assignments out of the graph.
RECIPE_HEADER = re.compile(
    r"(?m)^(?P<name>[A-Za-z_][\w-]*)(?P<params>[^:\n]*):(?![=])(?P<deps>[^\n]*)$"
)
def read_justfile(root: Path = ROOT) -> str:
    return (root / "justfile").read_text(encoding="utf-8")


def recipe_body(justfile: str, name: str) -> str:
    """The header line plus every indented line beneath it."""
    lines = justfile.splitlines()
    start = next(
        index
        for index, line in enumerate(lines)
        if line.startswith((f"{name}:", f"{name} "))
    )
    end = len(lines)
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line and not line.startswith((" ", "\t", "#")):
            end = index
            break
    return "\n".join(lines[start:end])


def shell_commands(shell: str):
    """Commands in shell position; comments, strings and spacing are inert."""
    return commands(parse_shell(shell, origin="justfile graph"))


def invokes(shell: str, program: str) -> bool:
    return any(command.program == program for command in shell_commands(shell))


def just_recipes(shell: str) -> tuple[str, ...]:
    return tuple(
        command.subcommand()
        for command in shell_commands(shell)
        if command.program == "just" and command.subcommand()
    )


def gate_commands(shell: str) -> tuple[str, ...]:
    found = []
    for command in shell_commands(shell):
        if "capsem-gate" not in command.argv:
            continue
        index = command.argv.index("capsem-gate") + 1
        if index < len(command.argv):
            found.append(command.argv[index])
    return tuple(found)


@functools.cache
def gate_command_body(name: str) -> str:
    """What a `capsem-gate` subcommand's plan would run.

    Recipes dispatch into Python now, so a resolver that only reads the
    justfile stops at the dispatch line and cannot see what the command
    actually does. Rendering the plan asks the same question one level down,
    and it costs nothing -- a plan is built without touching the machine.
    """
    import argparse

    root = ROOT
    try:
        from capsem_builder.gate import cli  # noqa: F401 - registers every command
        from capsem_builder.gate.command import GateCommand
        from capsem_builder.gate.proc import Runner

        command = GateCommand.registry.get(name)
        if command is None:
            return ""
        return (
            command(
                Runner(root),
                argparse.Namespace(dry_run=False, graph=False, timing=False),
            )
            .plan()
            .describe()
        )
    except Exception:
        # A plan that cannot be built here says nothing about what it runs;
        # the caller falls back to the justfile graph.
        return ""


def recipe_dependency_graph(justfile: str) -> dict[str, tuple[str, ...]]:
    return {
        match.group("name"): tuple(match.group("deps").split())
        for match in RECIPE_HEADER.finditer(justfile)
    }


def recipes_reaching(justfile: str, program: str) -> frozenset[str]:
    """Every recipe that runs ``program`` itself, or reaches one that does."""
    graph = recipe_dependency_graph(justfile)
    bodies = {name: recipe_body(justfile, name) for name in graph}
    # A recipe reaches `command` if it runs it, or if it dispatches to a gate
    # subcommand whose plan runs it. Without the second clause the walk stops
    # at every `uv run --project build_system --frozen capsem-gate ...` line, which is now most of them.
    reaching = {
        name
        for name, body in bodies.items()
        if invokes(body, program) or _gate_dispatch_matches(body, program)
    }
    changed = True
    while changed:
        changed = False
        for name, dependencies in graph.items():
            if name in reaching:
                continue
            invoked = set(just_recipes(bodies[name]))
            if any(dep in reaching for dep in dependencies) or any(
                call in reaching for call in invoked
            ):
                reaching.add(name)
                changed = True
    return frozenset(reaching)


def _gate_dispatch_matches(body: str, program: str) -> bool:
    return any(invokes(gate_command_body(name), program) for name in gate_commands(body))


def recipes_running_pnpm(justfile: str) -> frozenset[str]:
    return recipes_reaching(justfile, "pnpm")


def recipe_reaches_pnpm_through_the_gate(body: str) -> bool:
    return any(invokes(gate_command_body(name), "pnpm") for name in gate_commands(body))


def shell_reaches_pnpm(shell: str, justfile: str) -> bool:
    """Whether a CI job's shell runs pnpm directly, through a recipe, or
    through a gate command a recipe dispatches to."""
    if invokes(shell, "pnpm"):
        return True
    if recipe_reaches_pnpm_through_the_gate(shell):
        return True

    reaching = recipes_running_pnpm(justfile)
    invoked = just_recipes(shell)
    if any(recipe in reaching for recipe in invoked):
        return True

    bodies = {name: recipe_body(justfile, name) for name in recipe_dependency_graph(justfile)}
    return any(
        recipe_reaches_pnpm_through_the_gate(bodies.get(recipe, "")) for recipe in invoked
    )
