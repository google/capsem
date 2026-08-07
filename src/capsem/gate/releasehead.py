"""The revision a release publishes is the one the gate qualified.

`release.py` captured `git rev-parse HEAD` while *building* its plan, and that
was wrong twice over.

It ran a command during `--dry-run`, through a `Runner` the module constructed
itself -- so the seal around plan construction never saw it, and neither did
the run log. The dry run printed a real revision while nothing recorded that
anything had run.

And it read the revision before the plan existed, so the value baked into the
confirmation came from whenever the description happened to be assembled rather
than from the run. A dry run therefore promised something about a revision the
eventual run would not be testing.

Both disappear when the capture is a step. `RecordHead` writes the revision
down when it runs; `ConfirmHead` reads what it wrote. The order between them is
the safety property: capture, qualify, re-assert. If the tree moves during a
forty-minute gate, the confirmation refuses rather than publishing a revision
nothing tested.
"""

from __future__ import annotations

from pathlib import Path

from .actions import Action
from .config import GateConfig
from .context import Context
from .errors import GateError
from .fileactions import write_text


def head_file(config: GateConfig) -> Path:
    """Where the qualified revision is written down between the two steps."""
    return config.path(config.release.preflight_dir) / "tested-head"


class RecordHead(Action, name="record-head"):
    """Write down the revision the gate is about to qualify."""

    def __init__(self, target: Path) -> None:
        self._target = target

    def render(self) -> str:
        return f"record the revision under test in {self._target.name}"

    def perform(self, context: Context) -> None:
        if context.observing:
            # Reading a release plan is not running one. A contract that runs
            # the plan to read back its argv would otherwise overwrite the
            # running gate's `tested-head` with the recording runner's empty
            # capture, and `confirm-head` -- one step before publishing, an
            # hour later -- refuses to publish a revision nothing recorded.
            return
        head = context.runner.capture(["git", "rev-parse", "HEAD"])
        write_text(self._target, head)
        context.journal.note(f"qualifying {head}")


class ConfirmHead(Action, name="confirm-head"):
    """Re-assert that the qualified revision is the one being published.

    Reads what `RecordHead` wrote rather than taking the value as an argument,
    because an argument would have to be known when the plan was built -- which
    is the defect this pair exists to remove.
    """

    def __init__(self, script: str, source: Path) -> None:
        self._script = script
        self._source = source

    def render(self) -> str:
        return f"uv run python {self._script} --expected-head $(cat {self._source.name})"

    def perform(self, context: Context) -> None:
        if not self._source.is_file():
            raise GateError(
                f"{self._source} is missing, so the revision this gate "
                f"qualified was never recorded; refusing to publish"
            )
        head = self._source.read_text(encoding="utf-8").strip()
        if not head:
            raise GateError(
                f"{self._source} is empty; refusing to publish a revision nothing recorded"
            )
        context.runner.script(self._script, "--expected-head", head)
