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
        if line.startswith(f"{name}:") or line.startswith(f"{name} ")
    )
    end = len(lines)
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line and not line.startswith((" ", "\t", "#")):
            end = index
            break
    return "\n".join(lines[start:end])


def recipe_dependency_graph(justfile: str) -> dict[str, tuple[str, ...]]:
    return {
        match.group("name"): tuple(match.group("deps").split())
        for match in RECIPE_HEADER.finditer(justfile)
    }


def recipes_reaching(justfile: str, command: re.Pattern[str]) -> frozenset[str]:
    """Every recipe that runs `command` itself, or reaches one that does."""
    graph = recipe_dependency_graph(justfile)
    bodies = {name: recipe_body(justfile, name) for name in graph}
    reaching = {name for name, body in bodies.items() if command.search(body)}
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


def recipes_running_pnpm(justfile: str) -> frozenset[str]:
    return recipes_reaching(justfile, PNPM_CALL)


def shell_reaches_pnpm(shell: str, justfile: str) -> bool:
    """Whether a CI job's shell runs pnpm directly or through a just recipe."""
    if PNPM_CALL.search(shell):
        return True
    reaching = recipes_running_pnpm(justfile)
    return any(recipe in reaching for recipe in JUST_RECIPE.findall(shell))
