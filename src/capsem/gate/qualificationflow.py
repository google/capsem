"""The one command-lifecycle seam for exact qualification reuse and resume."""

from __future__ import annotations

import json
import os
from dataclasses import dataclass
from pathlib import Path

from . import prefix, qualificationevidence
from .config import GateConfig
from .errors import GateError
from .plan import Plan
from .qualificationevidence import (
    QualificationEvidence,
    QualificationPolicy,
    ResumeEvidence,
)
from .runlog import RunLog
from .runlogschema import QualificationComplete, QualificationResume, QualificationReuse
from .sourcecommit import SourceCommit, require_detached_checkout, require_local_main


@dataclass(frozen=True)
class Decision:
    """Evidence selected before any prefix, sandbox, lock, or plan action."""

    complete: QualificationEvidence | None
    resumed: ResumeEvidence | None
    carried: frozenset[str]
    reuse: Path | None
    child_arguments: tuple[str, ...] = ()

    @property
    def shortcut(self) -> bool:
        return self.complete is not None and self.resumed is None


def progress(
    decision: Decision, commit: SourceCommit | None, requested_frontier: str | None
) -> str | None:
    """One human-readable explanation of reused work and its authority."""
    if decision.resumed is not None:
        parent = decision.resumed.parent
        return (
            f"resuming {commit} at {decision.resumed.frontier} from {parent.run_id}: "
            f"{parent.run_log} ({parent.digest})"
        )
    if decision.carried:
        return f"carrying {len(decision.carried)} steps before {requested_frontier}"
    return None


def _retained_prefix(config: GateConfig, commit: SourceCommit) -> Path:
    selected = prefix.for_source_commit(config, commit)
    if selected.is_symlink() or not selected.is_dir():
        raise GateError(
            f"partial qualification for {commit} has no retained exact prefix at {selected}"
        )
    if selected.stat().st_uid != os.getuid():
        raise GateError(f"retained exact prefix {selected} has the wrong owner")
    require_detached_checkout(selected, commit)
    return selected


def decide(
    config: GateConfig,
    *,
    policy: QualificationPolicy,
    commit: SourceCommit | None,
    plan: Plan,
    args,
    carried: frozenset[str],
    reuse_path: Path | None,
) -> Decision:
    """Resolve complete and partial evidence without trusting command prose."""
    if policy is QualificationPolicy.NONE or commit is None:
        return Decision(None, None, carried, reuse_path)

    history = qualificationevidence.authority(config)
    require_local_main(history.root, commit)
    complete = qualificationevidence.find_complete(history, commit)
    if policy is QualificationPolicy.REQUIRE:
        if complete is None:
            if getattr(args, "force", "false") == "true":
                # The second gate `--force` has to reach. The plan swaps its
                # accept step for a recorded waiver, and this refuses before any
                # plan runs, so relaxing only one of them leaves the flag
                # looking broken. The policy itself is deliberately not
                # downgraded here: it is what decides sandbox enforcement, and
                # forcing a release must not quietly unseal the sandbox too.
                return Decision(None, None, carried, reuse_path)
            raise GateError(
                f"source commit {commit} has no complete exact qualification run log; "
                f"run `just test {commit}` first"
            )
        return Decision(complete, None, carried, reuse_path)
    if complete is not None:
        return Decision(complete, None, frozenset(), None)

    partial = qualificationevidence.find_resume(history, commit, plan)
    requested = getattr(args, "resume_from", None)
    named = getattr(args, "prefix", None)
    if requested is not None or named is not None:
        if partial is None or requested != partial.frontier or carried != partial.carried:
            raise GateError(
                f"exact-source continuation for {commit} is not supported by its latest "
                "partial run log; omit --prefix/--from to select the proven frontier"
            )
        retained = _retained_prefix(config, commit)
        if reuse_path is None or reuse_path.absolute() != retained.absolute():
            raise GateError(f"exact-source continuation must use retained prefix {retained}")
        return Decision(None, partial, carried, retained)
    if partial is None:
        return Decision(None, None, carried, reuse_path)

    retained = _retained_prefix(config, commit)
    return Decision(
        None,
        partial,
        partial.carried,
        retained,
        ("--prefix", str(retained), "--from", partial.frontier),
    )


def begin(
    log, decision: Decision, commit: SourceCommit | None, policy: QualificationPolicy
) -> None:
    """Write the selected lineage before the graph starts."""
    if not isinstance(log, RunLog) or commit is None:
        return
    if policy is QualificationPolicy.REUSE_OR_RUN:
        log.qualification_attempt(commit)
    if decision.complete is not None:
        log.emit(
            QualificationReuse(
                source_commit=str(commit),
                qualification=decision.complete.reference,
            )
        )
    if decision.resumed is not None:
        log.emit(
            QualificationResume(
                source_commit=str(commit),
                parent=decision.resumed.parent,
                carried_steps=tuple(sorted(decision.resumed.carried)),
            )
        )


def finish(
    log,
    config: GateConfig,
    commit: SourceCommit | None,
    policy: QualificationPolicy,
    plan: Plan,
    decision: Decision,
) -> None:
    """Claim completion only after the whole graph returned successfully."""
    if (
        not isinstance(log, RunLog)
        or commit is None
        or policy is not QualificationPolicy.REUSE_OR_RUN
    ):
        return
    receipt = config.path(config.candidate.source_state_file)
    try:
        state = json.loads(receipt.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise GateError(f"cannot read exact-source qualification receipt {receipt}") from error
    if state.get("source_kind") != "commit" or state.get("source_commit") != commit:
        raise GateError(f"source receipt does not bind qualification to {commit}")
    digest = state.get("digest")
    if not isinstance(digest, str) or len(digest) != 64:
        raise GateError("source receipt has no canonical digest")
    log.emit(
        QualificationComplete(
            source_commit=str(commit),
            source_digest=digest,
            plan_digest=qualificationevidence.plan_digest(plan.labels, plan.edges),
            parent=None if decision.resumed is None else decision.resumed.parent,
        )
    )
