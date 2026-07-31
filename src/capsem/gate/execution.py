"""A named unit of gate work: some actions, what it may not share, what it makes.

Deliberately concrete. A `Step` has no behaviour of its own beyond running its
actions in order and recording what it produced, so there is nothing to
subclass. Polymorphism lives one level down in `Action`, where the variation
actually is; a `Step` subclass would only ever be a closure wearing a class,
which is the shape this replaces.

Ordering is deliberately absent. A step does not know what comes before it --
that is `plan`, which holds the edges. Keeping it out means the same step can
be reused by a command that sequences it differently, which is the whole reason
the six test modules can share one pytest step instead of eleven near-copies.

`contends` names what this step may not share, drawn from
`[execution.exclusives]` where each entry carries the reason it exists.
`produces` names the artifacts whose bytes the run log should record, so a run
can answer "which bytes did this build" after the tree is gone.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from .actions import Action
from .context import Context
from .fileactions import Hash
from .harnessschema import Exclusive


@dataclass(frozen=True)
class Step:
    """One named unit of gate work."""

    label: str
    actions: tuple[Action, ...]
    contends: tuple[Exclusive, ...] = ()
    produces: tuple[Path, ...] = field(default_factory=tuple)

    def render(self) -> list[str]:
        """One line per action, for the dry run."""
        return [action.render() for action in self.actions]

    def run(self, context: Context) -> None:
        """Every action in order, then record what came out.

        Stops at the first failing action: the ones after it were written
        against what it was supposed to produce.
        """
        for action in self.actions:
            with context.journal.action(action):
                action.perform(context)
        for artifact in self.produces:
            Hash(artifact).perform(context)


def step(
    label: str,
    *actions: Action,
    contends: tuple[Exclusive, ...] = (),
    produces: tuple[Path, ...] = (),
) -> Step:
    """Build a step from actions given positionally, which reads better.

        step("sign", Run([...]), Run([...]))

    rather than passing a tuple, because at every call site the actions are
    written out literally and the extra brackets are noise.
    """
    return Step(label=label, actions=actions, contends=contends, produces=produces)
