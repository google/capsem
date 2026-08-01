"""What a plan must hold before a machine lock is taken.

Separate from `plan` for the same reason `planreport` is: a plan's job is to
hold steps and edges and run them. How it explains itself is one other
responsibility, and what makes it well-formed is a third.

Every check here is total and costs nothing, which is why they all run up
front. Discovering any of them thirty minutes into a held lock costs the thirty
minutes and leaves a half-built tree behind -- and each one of these describes
a mistake that reads perfectly at the call site.
"""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING

from .config import GateConfig
from .errors import GateError

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from .execution import Step
    from .plan import Plan


def validate(plan: Plan, config: GateConfig) -> None:
    """Everything that must be true before the plan is allowed to run."""
    plan.order()  # raises naming the cycle, before anything else looks at it
    require_declared_exclusives(plan, config)
    require_one_owner_per_artifact(plan)


def require_declared_exclusives(plan: Plan, config: GateConfig) -> None:
    """A claim on something absent from config excludes nothing.

    `[execution.exclusives]` carries the reason each one exists, and that is
    the part worth having. An invented name reads at the call site exactly like
    a real constraint -- `contends=(Exclusive("apple_vm"),)` beside a step that
    launches VMs looks right -- and enforces nothing whatsoever.
    """
    declared = set(config.execution.exclusives)
    for step in plan.steps:
        for resource in step.contends:
            if resource.name in declared:
                continue
            raise GateError(
                f"{step.label!r} in the {plan.name} plan contends for "
                f"{resource.name!r}, which is not in [execution.exclusives]. "
                f"Declare it there with the reason it exists, or claim one of: "
                f"{', '.join(sorted(declared))}"
            )


def require_one_owner_per_artifact(plan: Plan) -> None:
    """Two steps writing one path must contend for the same thing.

    A lock around the mutation is not a lock around the artifact. A step can
    hold an exclusive while it builds, release it, and hand back "look at this
    path" -- and the next claimant overwrites that path before the consumer
    reads it. The edge orders the consumer after *its* producer and says
    nothing about a second producer running beside it.

    This is how four helpers came to lock `astro build`, release, and then read
    a `dist/` the next build had already replaced. Caught when the plan is
    built, because by the time it happens the evidence is gone.
    """
    owners: dict[Path, list[Step]] = {}
    for step in plan.steps:
        for artifact in step.produces:
            owners.setdefault(artifact, []).append(step)

    for artifact, producers in sorted(owners.items()):
        if len(producers) < 2:
            continue
        shared = set.intersection(
            *({resource.name for resource in step.contends} for step in producers)
        )
        if shared:
            continue
        raise GateError(
            f"{len(producers)} steps in the {plan.name} plan write {artifact} "
            f"and share no exclusive: "
            f"{', '.join(sorted(step.label for step in producers))}. "
            f"Give them one, or have each produce its own path -- an edge "
            f"orders a consumer after its producer, not after every other."
        )
