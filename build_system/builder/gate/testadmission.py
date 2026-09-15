"""Adapter from candidate Git/history evidence to cache-owned admission policy."""

from __future__ import annotations

import argparse
import time
from typing import Protocol

from ..cache.admission import decide_admission
from ..cache.config import load_policy
from ..cache.gitimpact import inspect_git
from ..cache.models import AdmissionEvent, AdmissionEventKind
from ..cache.operations import last_admission_event, record_admission_event
from ..cache.paths import CachePaths
from . import qualificationevidence, qualificationjournal, runhistory
from .config import GateConfig
from .errors import GateError
from .proc import Runner
from .runlogschema import OK
from .sourcecommit import SourceCommit, source_commit_for_checkout


class CandidateLike(Protocol):
    """The narrow candidate state admission consumes."""

    _args: argparse.Namespace
    _config: GateConfig
    _runner: Runner


def _identity(command: CandidateLike, commit: SourceCommit | None) -> str:
    return str(commit or source_commit_for_checkout(command._config.root))


def _working_tree_history(config: GateConfig) -> tuple[bool, str | None]:
    """Whether the last working-tree candidate failed, and the HEAD of the last green one.

    `just test` without a commit never reaches the exact-source archive, so
    admission read no attempt and no baseline for it and admitted every run on
    a branch as the first -- fifteen full runs in a row, each after the last
    failed. The run history records the same facts. A run with no end that no
    one is writing was interrupted, which counts as failed.
    """
    running = runhistory.live(config)
    failed: bool | None = None
    for directory in runhistory.runs(config):
        if directory.name in running:
            continue
        try:
            events = runhistory.read(directory, config.runlog)
        except ValueError:
            events = []
        start = next((event for event in events if event.get("event") == "run.start"), None)
        if start is None or start.get("command") != "candidate" or start.get("source_commit"):
            continue
        end = next((event for event in reversed(events) if event.get("event") == "run.end"), None)
        green = end is not None and end.get("status") == OK
        if failed is None:
            failed = not green
        if green:
            return failed, str(start["head"])
    return bool(failed), None


def admit(command: CandidateLike, commit: SourceCommit | None) -> None:
    """Refuse wasteful complete proof before the machine lock and plan actions."""
    if command._runner.observing:
        return
    cache_policy = load_policy(command._config.root)
    authority = qualificationevidence.authority(command._config)
    paths = CachePaths(repository_root=authority.root, policy=cache_policy)
    target = _identity(command, commit)
    if commit is None:
        attempt_failed, baseline = _working_tree_history(authority)
    else:
        latest = qualificationevidence.latest_complete(authority)
        attempt = qualificationjournal.latest_attempt(authority)
        attempt_failed = attempt is not None and attempt.end.status != OK
        baseline = None if latest is None else str(latest[0])
    if baseline is None:
        changed_paths: tuple[str, ...] = ()
        commits = 0
    else:
        impact = inspect_git(command._config.root, baseline, None if commit is None else target)
        changed_paths = impact.paths if impact.ancestor else ()
        commits = impact.commits
    prior = last_admission_event(paths.root, cache_policy.test_admission.state_path)
    forced = getattr(command._args, "mode", "normal") == "force"
    decision = decide_admission(
        policy=cache_policy.test_admission,
        baseline=baseline,
        target=target,
        changed_paths=changed_paths,
        commits_since_success=commits,
        forced=forced,
        force_reason=getattr(command._args, "reason", ""),
        prior_forced=prior is not None and prior.kind is AdmissionEventKind.FORCED_ATTEMPT,
        failed_attempt=attempt_failed,
    )
    if not decision.allowed:
        routes = "\n".join(f"  just focus-test {group}" for group in decision.groups)
        suffix = f"\nUse the owning rails instead:\n{routes}" if routes else ""
        raise GateError(
            f"complete test refused: {decision.explanation}{suffix}\n"
            "Release commands self-qualify; a local full run is optional. "
            f'An explicitly approved exception uses: just test {target} force "<reason>"'
        )
    command._runner.note(f"complete test admitted: {decision.explanation}")
    if decision.forced:
        record_admission_event(
            paths.root,
            cache_policy.test_admission.state_path,
            AdmissionEvent(
                kind=AdmissionEventKind.FORCED_ATTEMPT,
                timestamp_ns=time.time_ns(),
                source_identity=target,
                reason=getattr(command._args, "reason", "").strip(),
            ),
        )


def complete(command: CandidateLike, commit: SourceCommit | None) -> None:
    """Reset the force rail only after successful non-forced complete proof."""
    if command._runner.observing or getattr(command._args, "mode", "normal") == "force":
        return
    cache_policy = load_policy(command._config.root)
    authority = qualificationevidence.authority(command._config)
    paths = CachePaths(repository_root=authority.root, policy=cache_policy)
    record_admission_event(
        paths.root,
        cache_policy.test_admission.state_path,
        AdmissionEvent(
            kind=AdmissionEventKind.COMPLETE_SUCCESS,
            timestamp_ns=time.time_ns(),
            source_identity=_identity(command, commit),
            reason="",
        ),
    )
