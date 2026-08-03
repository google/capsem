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
import os
from pathlib import Path

from .actions import Action
from .context import Context
from .errors import GateError
from .fileactions import write_text


def _record_file(context: Context) -> Path:
    return context.path(context.config.candidate.source_state_file)


def gate_source() -> Path:
    """Where the gate's own code is being imported from.

    `HEAD` and the digest describe a checkout. Neither says the code building
    this plan came from that checkout, and an installed or vendored copy would
    let the gate measure one tree while qualifying another.
    """
    from . import config as _module  # any module of the package answers this

    return Path(_module.__file__).resolve().parent


def _measure(context: Context) -> dict[str, str]:
    return {
        "head": context.runner.capture(["git", "rev-parse", "HEAD"]),
        "digest": context.runner.capture(
            [
                "uv", "run", "python",
                str(context.path(context.config.candidate.source_digest_script)),
            ]
        ),
        "gate_source": str(gate_source()),
    }


class RecordSourceState(Action, name="record-source-state"):
    """Write down the source state this gate is about to qualify."""

    def render(self) -> str:
        return "record the HEAD and source digest under test"

    def perform(self, context: Context) -> None:
        state = _measure(context)
        write_text(_record_file(context), json.dumps(state))
        context.journal.note(f"testing source state {state['digest']} at {state['head']}")


class RequireIsolatedBytecode(Action, name="require-isolated-bytecode"):
    """The recorded digest is of the bytes on disk. Prove those are the ones
    the interpreter is running.

    CPython validates a `.pyc` against the source's mtime and size, so two
    edits of the same length inside one timestamp tick leave bytecode that
    still looks current. `capsem.gatelaunch` closes that by re-execing under a
    per-invocation cache prefix before importing any of this package -- and
    exports a marker saying so, which is the only thing a running gate can
    check about how it was started.

    A step rather than a rule inside `RecordSourceState`, because they are two
    claims: one is what the tree contains, the other is what this process is
    executing.
    """

    def render(self) -> str:
        return "check this interpreter cannot be running stale bytecode"

    def perform(self, context: Context) -> None:
        from capsem.gatelaunch import MARKER, PYCACHE

        prefix = os.environ.get(MARKER)
        if not prefix:
            raise GateError(
                "this gate was not started through capsem-gate, so its bytecode "
                "cache is the ambient one. A same-size edit within one timestamp "
                f"tick leaves a stale .pyc that still validates, and {MARKER} is "
                "how a run proves it re-execed under a private cache first. Run "
                "`uv run capsem-gate ...`, or export it with a fresh directory."
            )
        context.journal.note(f"{PYCACHE}={prefix}")


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
