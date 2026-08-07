#!/usr/bin/env python3
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
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# A recipe header is `name params: deps`, anchored at column zero so recipe
# bodies (always indented) cannot match. `:(?![=])` keeps `foo := bar`
# assignments out of the graph.
RECIPE_HEADER = re.compile(
    r"(?m)^(?P<name>[A-Za-z_][\w-]*)(?P<params>[^:\n]*):(?![=])(?P<deps>[^\n]*)$"
)
# `just`/`pnpm` must be followed by an argument on the SAME line: `\s` would
# span newlines and match the trailing "just" of `uses: extractions/setup-just`.
JUST_CALL = re.compile(r"(?<![\w-])just[ \t]+[-_A-Za-z]")
JUST_RECIPE = re.compile(r"(?<![\w-])just[ \t]+([-_A-Za-z][\w-]*)")
PNPM_CALL = re.compile(r"(?<![\w-])pnpm[ \t]")


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


GATE_COMMAND = re.compile(r"capsem-gate\s+([a-z][a-z0-9-]*)")


@functools.cache
def gate_command_body(name: str) -> str:
    """What a `capsem-gate` subcommand's plan would run.

    Recipes dispatch into Python now, so a resolver that only reads the
    justfile stops at the dispatch line and cannot see what the command
    actually does. Rendering the plan asks the same question one level down,
    and it costs nothing -- a plan is built without touching the machine.
    """
    import argparse
    import pathlib
    import sys

    root = pathlib.Path(__file__).resolve().parents[1]
    if str(root / "src") not in sys.path:
        sys.path.insert(0, str(root / "src"))
    try:
        from capsem.gate import cli  # noqa: F401 - registers every command
        from capsem.gate.command import GateCommand
        from capsem.gate.proc import Runner

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


def recipes_reaching(justfile: str, command: re.Pattern[str]) -> frozenset[str]:
    """Every recipe that runs `command` itself, or reaches one that does."""
    graph = recipe_dependency_graph(justfile)
    bodies = {name: recipe_body(justfile, name) for name in graph}
    # A recipe reaches `command` if it runs it, or if it dispatches to a gate
    # subcommand whose plan runs it. Without the second clause the walk stops
    # at every `uv run capsem-gate ...` line, which is now most of them.
    reaching = {
        name
        for name, body in bodies.items()
        if command.search(body) or _gate_dispatch_matches(body, command)
    }
    changed = True
    while changed:
        changed = False
        for name, dependencies in graph.items():
            if name in reaching:
                continue
            invoked = set(JUST_RECIPE.findall(bodies[name]))
            if any(dep in reaching for dep in dependencies) or any(
                call in reaching for call in invoked
            ):
                reaching.add(name)
                changed = True
    return frozenset(reaching)


def _gate_dispatch_matches(body: str, command: re.Pattern[str]) -> bool:
    return any(
        command.search(gate_command_body(name)) for name in GATE_COMMAND.findall(body)
    )


def recipes_running_pnpm(justfile: str) -> frozenset[str]:
    return recipes_reaching(justfile, PNPM_CALL)


def recipe_reaches_pnpm_through_the_gate(body: str) -> bool:
    return any(
        PNPM_CALL.search(gate_command_body(name))
        for name in GATE_COMMAND.findall(body)
    )


def shell_reaches_pnpm(shell: str, justfile: str) -> bool:
    """Whether a CI job's shell runs pnpm directly, through a recipe, or
    through a gate command a recipe dispatches to."""
    if PNPM_CALL.search(shell):
        return True
    if recipe_reaches_pnpm_through_the_gate(shell):
        return True

    reaching = recipes_running_pnpm(justfile)
    invoked = JUST_RECIPE.findall(shell)
    if any(recipe in reaching for recipe in invoked):
        return True

    bodies = {name: recipe_body(justfile, name) for name in recipe_dependency_graph(justfile)}
    return any(
        recipe_reaches_pnpm_through_the_gate(bodies.get(recipe, "")) for recipe in invoked
    )
