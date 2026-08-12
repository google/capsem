"""Continuing a failed run instead of repeating it.

A gate run is long and the graph is deep, so a fix to a step near the end used
to cost a full replay of everything before it. Six consecutive `just test` runs
were spent that way: each stopped one step later than the last, and each paid
twenty-odd minutes to reach the new frontier. The private checkout made it
worse rather than better -- a fresh copy per run starts with no `target/`, so
every replay is cold.

Resuming names two things: the prefix to work in, which still holds the earlier
run's build output, and the step to start at. Everything the graph puts *before*
that step is carried; that step and everything after it runs.

    capsem-gate candidate --prefix ~/.cg/a025fce7 --from artifacts.build-chain

Derived from the graph rather than from a previous run's log, so the answer is
the same every time and can be checked before anything executes -- what comes
before a step is a property of the plan, not of what happened to succeed last
night.

**This is an iteration tool and never a qualification.** `AGENTS.md` and
`release-process` forbid a reduced gate, a skip flag, or an environment bypass
on the release path, and a resumed run is all three if it is allowed to stand
in for a clean one. Three things keep it honest:

  it is refused outright when the run is qualifying a release

  every carried step is recorded as `carried`, never `ok`, so the run log says
  which steps this process actually ran

  the prefix must be one this gate made, under the configured root, so a
  mistyped path cannot have the gate build into somebody's working tree
"""

from __future__ import annotations

from pathlib import Path

from . import prefix
from .config import GateConfig
from .errors import GateError
from .execution import ResumePolicy


def ancestors(plan, label: str) -> frozenset[str]:
    """Every step that must finish before `label`, transitively.

    This is what `--from` carries. Derived from the graph rather than from a
    previous run's log, which makes it answerable before anything runs and
    identical every time: the steps before a given one are a property of the
    plan, not of what happened to succeed last night.
    """
    if label not in plan.labels:
        near = sorted(name for name in plan.labels if label in name)
        raise GateError(
            f"no step named {label!r} in the {plan.name} plan"
            + (f"; did you mean {near}?" if near else "")
        )

    seen: set[str] = set()
    frontier = list(plan.after_of(label))
    while frontier:
        current = frontier.pop()
        if current in seen:
            continue
        seen.add(current)
        frontier.extend(plan.after_of(current))
    return frozenset(seen)


def carried(plan, config: GateConfig, label: str | None, *, qualifying: bool) -> frozenset[str]:
    """The steps a `--from` run may skip, with the refusals that keep it honest."""
    if label is None:
        return frozenset()
    if qualifying:
        raise GateError(
            "--from cannot be used while qualifying a release. It carries steps "
            "this process did not execute, which is exactly the reduced gate "
            "the release process forbids."
        )
    del config
    return frozenset(
        step_label
        for step_label in ancestors(plan, label)
        if plan.step_named(step_label).resume is ResumePolicy.REUSE
    )


def existing(config: GateConfig, given: str) -> Path:
    """The prefix `--prefix` names, checked to be one this gate made.

    Fenced on the configured root for the same reason `prefix.reclaim` is: a
    run is about to build in it, and pointing this at a checkout would have the
    gate write its artifacts into somebody's working tree.
    """
    root = prefix.parent_dir(config).resolve()
    tree = Path(given).expanduser().resolve()
    if tree.parent != root:
        raise GateError(f"{tree} is not a prefix under {root}")
    if not tree.is_dir():
        raise GateError(f"prefix {tree} does not exist; run without --prefix to make one")
    return tree


def resolve(
    plan, config: GateConfig, args, *, qualifying: bool
) -> tuple[frozenset[str], Path | None]:
    """What a run was told to carry, and where it was told to work.

    Both flags in one call because they are one idea and one refusal surface,
    and because resolving them separately at the call site is how the release
    check ends up guarding only half of it.

    Read with `getattr`: the flags hang off the shared parser, so every real
    invocation has them, while the fifty-odd hand-built `Namespace` objects in
    the test suite do not and should not have to.
    """
    carried_steps = carried(plan, config, getattr(args, "resume_from", None), qualifying=qualifying)
    named = getattr(args, "prefix", None)
    return carried_steps, existing(config, named) if named else None
