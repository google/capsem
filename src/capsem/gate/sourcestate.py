"""What the gate qualified, recorded before it starts and re-asserted after.

A forty-minute gate that qualified a HEAD nobody has, or a working tree edited
halfway through, has proved something about no particular version of the
software. Both are captured at the start and compared at the end.

Two granularities, because they answer different questions. `HEAD` catches a
commit or a checkout landing mid-run. The source digest covers tracked *and*
untracked non-ignored bytes, which is what ordinary uncommitted development
looks like -- so the gate supports a dirty tree and still fails if that tree
changes underneath it.

Recorded by a step rather than read while the plan is built, for the same
reason as the release head: reading it during construction runs a command
during `--dry-run`, and freezes whatever was checked out when the description
was assembled rather than what the run is testing.
"""

from __future__ import annotations

import json
from pathlib import Path

from .actions import Action
from .context import Context
from .errors import GateError
from .fileactions import write_text


def _record_file(context: Context) -> Path:
    return context.path(context.config.candidate.source_state_file)


def _measure(context: Context) -> dict[str, str]:
    return {
        "head": context.runner.capture(["git", "rev-parse", "HEAD"]),
        "digest": context.runner.capture(
            [
                "uv", "run", "python",
                str(context.path(context.config.candidate.source_digest_script)),
            ]
        ),
    }


class RecordSourceState(Action, name="record-source-state"):
    """Write down the source state this gate is about to qualify."""

    def render(self) -> str:
        return "record the HEAD and source digest under test"

    def perform(self, context: Context) -> None:
        state = _measure(context)
        write_text(_record_file(context), json.dumps(state))
        context.journal.note(f"testing source state {state['digest']} at {state['head']}")


class RequireSourceUnchanged(Action, name="require-source-unchanged"):
    """Whatever passed must be what was measured, at both granularities."""

    def render(self) -> str:
        return "check the HEAD and source digest still match what was recorded"

    def perform(self, context: Context) -> None:
        recorded = _record_file(context)
        if not recorded.is_file():
            raise GateError(
                f"{recorded} is missing, so the source state this gate ran "
                "against was never recorded"
            )

        before = json.loads(recorded.read_text(encoding="utf-8"))
        after = _measure(context)

        if before["head"] != after["head"]:
            raise GateError(
                f"source HEAD changed while the gate was running: "
                f"{before['head']} -> {after['head']}"
            )
        if before["digest"] != after["digest"]:
            context.journal.note(
                f"before={before['digest']} after={after['digest']}"
            )
            context.runner.run(["git", "status", "--short"], check=False)
            raise GateError("the gate changed the source working tree")

        context.journal.note(f"verified source state {after['digest']}")
