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
from .config import GateConfig
from .context import Context
from .errors import GateError
from .execution import Kind, Needs, ResumePolicy, Speed, Step, step
from .fileactions import write_text
from .sourcecapture import CaptureSourceSnapshot
from .sourcecommit import SourceCommit, require_detached_checkout


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


def _digest(context: Context, tree: Path | None = None) -> str:
    """The source digest of a tree; this run's own when none is named."""
    argv = ["uv", "run", "python", str(context.path(context.config.candidate.source_digest_script))]
    if tree is not None:
        argv += ["--root", str(tree)]
    return context.runner.capture(argv)


def _measure(context: Context) -> dict[str, str]:
    """The state of both trees this run is about.

    Under a prefix there are two: the copy the gate reads, and the checkout it
    was copied from. Only the second can move -- the copy is frozen the moment
    it is made -- so measuring the copy alone proves the gate did not edit its
    own tree and proves *nothing* about the branch being qualified. Without the
    source half, a commit landing on `main` mid-run leaves every comparison
    passing and the gate publishes a qualification for a revision it never
    tested.

    Both granularities on both trees, because a checkout can be edited without
    `HEAD` moving at all. That is not an exotic case: the gate deliberately
    supports uncommitted work, so an ordinary save during a forty-minute run
    changes the digest and nothing else.

    Absent a prefix the two trees are one, so each pair is measured once and
    reported twice rather than hashing 2500 files a second time to reach the
    same answer.
    """
    from . import prefix

    selected = os.environ.get(context.config.environment.source_commit)
    if selected is not None:
        try:
            commit = SourceCommit(selected)
        except ValueError as error:
            raise GateError("release source marker is not a canonical commit") from error
        expected = prefix.for_source_commit(context.config, commit)
        if context.config.root.absolute() != expected.absolute():
            raise GateError(f"release source {commit} is not running from {expected}")
        require_detached_checkout(context.config.root, commit)
        digest = _digest(context)
        return {
            "source_kind": "commit",
            "source_commit": str(commit),
            "head": str(commit),
            "digest": digest,
            "gate_source": str(gate_source()),
        }

    source = prefix.source_checkout(context.config)
    head = context.runner.capture(["git", "rev-parse", "HEAD"])
    digest = _digest(context)
    return {
        "source_kind": "working-tree",
        "head": head,
        "source_head": (
            head
            if source is None
            else context.runner.capture(["git", "-C", str(source), "rev-parse", "HEAD"])
        ),
        "digest": digest,
        "source_digest": digest if source is None else _digest(context, source),
        "gate_source": str(gate_source()),
    }


class RecordSourceState(Action, name="record-source-state"):
    """Write down the source state this gate is about to qualify."""

    def render(self) -> str:
        return "record the HEAD and source digest under test"

    def perform(self, context: Context) -> None:
        if context.observing:
            return
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

        if context.observing:
            return
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

        if before.get("source_kind") != after.get("source_kind"):
            raise GateError("the gate's source identity mode changed while it was running")
        if before.get("source_kind") == "commit":
            if before.get("source_commit") != after.get("source_commit"):
                raise GateError(
                    "the exact release source changed while the gate was running: "
                    f"{before.get('source_commit')} -> {after.get('source_commit')}"
                )
            if before["digest"] != after["digest"]:
                raise GateError("the gate changed the exact release source working tree")
            context.journal.note(
                f"verified exact release source {after['source_commit']} at {after['digest']}"
            )
            return

        if before["head"] != after["head"]:
            raise GateError(
                f"source HEAD changed while the gate was running: "
                f"{before['head']} -> {after['head']}"
            )
        # The half a private copy would otherwise swallow. Unprefixed these are
        # the same comparisons again and cost nothing; prefixed they are the
        # only ones that can still see the tree under qualification move.
        if before.get("source_head") != after.get("source_head"):
            raise GateError(
                "the checkout this run was copied from moved while the gate was "
                f"running: {before.get('source_head')} -> {after.get('source_head')}"
            )
        # This one first, and the source's after it. Unprefixed the two are the
        # same measurement, and whichever is checked first writes the message
        # an operator reads -- "the gate changed the source working tree" is
        # the accurate one when there is no copy to have been edited from
        # outside.
        if before["digest"] != after["digest"]:
            context.journal.note(f"before={before['digest']} after={after['digest']}")
            context.runner.run(["git", "status", "--short"], check=False)
            raise GateError("the gate changed the source working tree")
        if before.get("source_digest") != after.get("source_digest"):
            raise GateError(
                "the checkout this run was copied from was edited while the gate "
                f"was running: {before.get('source_digest')} -> {after.get('source_digest')}"
            )

        context.journal.note(f"verified source state {after['digest']}")


def record_step(config: GateConfig) -> Step:
    """The one source identity boundary shared by every composed consumer."""
    return step(
        "source.record",
        RequireIsolatedBytecode(),
        RecordSourceState(),
        CaptureSourceSnapshot(),
        # The state file records the snapshot digest. `produces` are hashed
        # files; the directory itself is re-hashed by every consumer.
        produces=(config.path(config.candidate.source_state_file),),
        resume=ResumePolicy.ALWAYS_RUN,
        kind=Kind.STATIC_TEST,
        needs=frozenset({Needs.DISK}),
        speed=Speed.FAST,
    )


def verify_step(*extra: Action) -> Step:
    """The one terminal source-identity check shared by complete proofs."""
    return step(
        "source.verify",
        RequireSourceUnchanged(),
        *extra,
        kind=Kind.STATIC_TEST,
        needs=frozenset({Needs.DISK}),
        speed=Speed.FAST,
    )
