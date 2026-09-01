"""Continuing a failed run instead of repeating it.

A gate run is long and the graph is deep, so a fix to a step near the end used
to cost a full replay of everything before it. Six consecutive `just test` runs
were spent that way: each stopped one step later than the last, and each paid
twenty-odd minutes to reach the new frontier. The private checkout made it
worse rather than better -- a fresh copy per run starts with no `cache/target/`, so
every replay is cold.

Resuming names two things: the prefix to work in, which still holds the earlier
run's build output, and the step to start at. Everything the graph puts *before*
that step is carried; that step and everything after it runs.

    capsem-gate candidate --prefix cache/worktrees/a025fce7 --from artifacts.build-chain

Derived from the graph rather than from a previous run's log, so the answer is
the same every time and can be checked before anything executes -- what comes
before a step is a property of the plan, not of what happened to succeed last
night.

**Reusing proven work is not a reduced candidate gate.** A *skipped* step is
work nobody did; a *carried* step is work recursively proven by archived
exact-source journals in this retained prefix. `find_complete` accepts that
lineage through `attempt.resumed.parent`, so a local candidate may resume
without pretending the child process ran every ancestor.

That authority does not extend to release attempts. Release CI and the two
public dispatch commands have no recursively verified continuation journal for
their short release graph. Deriving carry from graph shape alone would let a
caller skip fresh qualification acceptance, remote-main validation, or mutable
channel resolution. Those runs therefore reject `--from`, `--prefix`, and
`--until`; their release-attempt edges always execute.

Two things keep it honest, and they are the ones that always did the work:

  every carried step is recorded as `carried`, never `ok`, so the run log says
  which steps this process actually ran

  the prefix must be one this gate made, under the configured root, so a
  mistyped path cannot have the gate build into somebody's working tree

`--from` is never empty. `auto` -- the default -- leaves the frontier to the
gate, which picks the deepest one a retained lineage proves. `scratch` carries
nothing, for when a local pass has to mean a pass on a cold runner. A step
name carries that step's ancestors. Resume has to be the default, or it is a
flag people remember only after paying for not remembering it.
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


def descendants(plan, label: str) -> frozenset[str]:
    """`label` and everything that waits on it, transitively.

    What `--until` carries, and the mirror of `ancestors`. A developer machine
    needs it: the glow-up purges the host, deletes `~/.capsem` and reinstalls
    from the channel under test, which is correct on a disposable runner and
    not something to do to a machine somebody is working on. Without a way to
    stop short, replaying the lane locally means running that.
    """
    if label not in plan.labels:
        near = sorted(name for name in plan.labels if label in name)
        raise GateError(
            f"no step named {label!r} in the {plan.name} plan"
            + (f"; did you mean {near}?" if near else "")
        )
    seen: set[str] = {label}
    frontier = [label]
    while frontier:
        current = frontier.pop()
        for candidate in plan.labels:
            if current in plan.after_of(candidate) and candidate not in seen:
                seen.add(candidate)
                frontier.append(candidate)
    return frozenset(seen)


def stopped(plan, label: str | None, *, qualifying: bool) -> frozenset[str]:
    """The steps a `--until` run may skip, with the same refusal as `--from`."""
    if label is None:
        return frozenset()
    if qualifying:
        raise GateError(
            "--until cannot be used while qualifying a release. It drops the "
            "steps a release exists to run, which is exactly the reduced gate "
            "the release process forbids."
        )
    return descendants(plan, label)


#: `--from` is never empty. These two are the values that are not step names.
AUTO = "auto"
SCRATCH = "scratch"


def explicit(label: str | None) -> str | None:
    """The frontier a caller actually named, or `None` for "you choose".

    `auto` is the default, so every command resumes unless told otherwise;
    reading it as "no request" is what lets the gate pick the deepest proven
    frontier instead of running everything again.
    """
    return None if label in (None, AUTO) else label


def carried(plan, config: GateConfig, label: str | None, *, qualifying: bool) -> frozenset[str]:
    """The steps a `--from` run may skip.

    A local candidate validates the derived set against its exact-source
    journal later in ``qualificationflow.decide``. A release attempt has no
    such lineage for its release graph, so it cannot name a frontier at all.
    """
    if label in (None, AUTO):
        return frozenset()
    if qualifying:
        raise GateError(
            "--from cannot be used while qualifying a release. Release attempts "
            "must freshly revalidate qualification and mutable channel state."
        )
    if label == SCRATCH:
        return frozenset()
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
    # `--until` joins the same set for the same reason: both name steps this
    # process will not execute, and a release refuses either.
    carried_steps |= stopped(plan, getattr(args, "stop_before", None), qualifying=qualifying)
    named = getattr(args, "prefix", None)
    if qualifying and named is not None:
        raise GateError(
            "--prefix cannot be used while qualifying a release. Release attempts "
            "have no retained continuation prefix authority."
        )
    return carried_steps, existing(config, named) if named else None
