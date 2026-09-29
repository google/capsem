from __future__ import annotations

import json
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path

import blake3

from . import auditfs, prefix, resume
from .actions import Action
from .config import GateConfig
from .context import Context
from .errors import GateError
from .plan import Plan
from .qualificationjournal import Attempt, load
from .runlogschema import (
    CARRIED,
    OK,
    PlanShape,
    QualificationRun,
)
from .sourcecommit import SourceCommit, tree_of


class QualificationPolicy(StrEnum):
    """How one command consumes exact-source qualification evidence."""

    NONE = "none"
    REUSE_OR_RUN = "reuse-or-run"
    REQUIRE = "require"


@dataclass(frozen=True)
class QualificationEvidence:
    reference: QualificationRun
    source_digest: str


@dataclass(frozen=True)
class ResumeEvidence:
    parent: QualificationRun
    frontier: str
    carried: frozenset[str]


def authority(config: GateConfig) -> GateConfig:
    """The checkout whose retained run history a private prefix consumes.

    Rebased onto the outer root, never re-parsed from it. A prefix runs the
    selected commit's gate code, so reading the working tree's
    `config/gate.toml` validated one tree's file against the other tree's
    schema: every key added on `main` made each already-qualified commit
    unreleasable, failing on a file the release does not need and cannot use.

    Only the outer *location* is wanted here, because the retained journals
    live there. Which paths to look in is the prefix's own business, and the
    prefix is the tree being released.
    """
    source = prefix.source_checkout(config)
    return config.model_copy(update={"root": Path(source)}) if source is not None else config


def archive_path(config: GateConfig, commit: SourceCommit, run_id: str) -> Path:
    return (
        config.path(config.runlog.root)
        / config.runlog.source_archive_dir
        / str(commit)
        / f"{run_id}.jsonl"
    )


def archive_attempt(config: GateConfig, commit: SourceCommit, directory: Path) -> Path:
    """Hard-link one live journal before its terminal event is appended."""
    source = directory / config.runlog.events
    target = archive_path(config, commit, directory.name)
    if target.is_symlink() or source.is_symlink():
        raise GateError("exact-source attempt journals must not be symlinks")
    archive = target.parent.parent
    if archive.is_symlink() or target.parent.is_symlink():
        raise GateError("exact-source journal archive directories must not be symlinks")
    if target.exists():
        if not target.samefile(source):
            raise GateError(f"qualification journal archive already exists at {target}")
        return target
    # The archive has to see the terminal events appended to the live journal,
    # so this must be a hardlink rather than a snapshot. Route it through the
    # one audited hardlink boundary; the source is generated output.
    auditfs.stage(source, target)
    if not target.samefile(source):
        raise GateError(f"qualification journal archive at {target} is not a hardlink")
    return target


def plan_digest(steps: tuple[str, ...], edges: tuple[tuple[str, str], ...]) -> str:
    encoded = json.dumps({"steps": steps, "edges": edges}, separators=(",", ":"))
    return blake3.blake3(encoded.encode()).hexdigest()


def _paths(config: GateConfig, commit: SourceCommit) -> tuple[Path, ...]:
    root = archive_path(config, commit, "attempt").parent
    if not root.is_dir() or root.is_symlink():
        return ()
    return tuple(sorted(root.glob("*.jsonl"), reverse=True))


def _matching(attempt: Attempt, commit: SourceCommit, shape: PlanShape | None) -> bool:
    return (
        attempt.start.command == "candidate"
        and attempt.start.source_commit == commit
        and attempt.start.head == commit
        and attempt.reused is None
        and (shape is None or attempt.shape == shape)
    )


def _parent(config: GateConfig, commit: SourceCommit, selected: QualificationRun) -> Attempt | None:
    expected = archive_path(config, commit, selected.run_id)
    if selected.path.absolute() != expected.absolute():
        return None
    attempt = load(config, expected)
    if attempt is None or attempt.reference.digest != selected.digest:
        return None
    return attempt


def _coverage(
    config: GateConfig,
    commit: SourceCommit,
    attempt: Attempt,
    shape: PlanShape,
    seen: frozenset[str] = frozenset(),
) -> frozenset[str] | None:
    if attempt.reference.run_id in seen or not _matching(attempt, commit, shape):
        return None
    covered = {label for label, status in attempt.steps.items() if status == OK}
    carried = {label for label, status in attempt.steps.items() if status == CARRIED}
    if not carried:
        return frozenset(covered) if attempt.resumed is None else None
    resumed = attempt.resumed
    if resumed is None or resumed.source_commit != commit or set(resumed.carried_steps) != carried:
        return None
    parent = _parent(config, commit, resumed.parent)
    if parent is None:
        return None
    inherited = _coverage(config, commit, parent, shape, seen | {attempt.reference.run_id})
    if inherited is None or not carried <= inherited:
        return None
    return frozenset(covered | carried)


def find_complete(config: GateConfig, commit: SourceCommit) -> QualificationEvidence | None:
    """Newest complete journal whose lineage covers every declared step."""
    for path in _paths(config, commit):
        attempt = load(config, path)
        if attempt is None or not _matching(attempt, commit, None):
            continue
        complete = attempt.complete
        if complete is None or complete.source_commit != commit:
            continue
        if complete.plan_digest != plan_digest(attempt.shape.steps, attempt.shape.edges):
            continue
        if attempt.end.status != OK or attempt.end.failures or attempt.end.skipped:
            continue
        if set(attempt.steps) != set(attempt.shape.steps):
            continue
        coverage = _coverage(config, commit, attempt, attempt.shape)
        if coverage != frozenset(attempt.shape.steps):
            continue
        parent = attempt.resumed.parent if attempt.resumed is not None else None
        if complete.parent != parent:
            continue
        if attempt.steps.get("source.record") != OK or attempt.steps.get("source.verify") != OK:
            continue
        return QualificationEvidence(attempt.reference, complete.source_digest)
    return None


def latest_complete(
    config: GateConfig,
) -> tuple[SourceCommit, QualificationEvidence] | None:
    """Newest valid complete candidate across retained source archives."""
    archive = config.path(config.runlog.root) / config.runlog.source_archive_dir
    if not archive.is_dir() or archive.is_symlink():
        return None
    latest: tuple[SourceCommit, QualificationEvidence] | None = None
    for directory in archive.iterdir():
        if not directory.is_dir() or directory.is_symlink():
            continue
        try:
            commit = SourceCommit(directory.name)
        except ValueError:
            continue
        evidence = find_complete(config, commit)
        if evidence is None:
            continue
        if latest is None or evidence.reference.run_id > latest[1].reference.run_id:
            latest = (commit, evidence)
    return latest


def find_resume(config: GateConfig, commit: SourceCommit, plan: Plan) -> ResumeEvidence | None:
    """Deepest graph frontier supported by one retained attempt lineage."""
    shape = PlanShape(steps=plan.labels, edges=plan.edges)
    best: tuple[int, ResumeEvidence] | None = None
    for path in _paths(config, commit):
        attempt = load(config, path)
        if attempt is None or not _matching(attempt, commit, shape):
            continue
        coverage = _coverage(config, commit, attempt, shape)
        if coverage is None:
            continue
        for label in plan.labels:
            carried = resume.carried(plan, config, label, qualifying=False)
            if carried and carried <= coverage:
                candidate = ResumeEvidence(attempt.reference, label, carried)
                score = len(carried)
                if best is None or score > best[0]:
                    best = (score, candidate)
    return None if best is None else best[1]


def find_release_proof(config: GateConfig, commit: SourceCommit) -> QualificationEvidence | None:
    """A passing `just test` for this commit, or for any commit with its tree.

    The exact commit first. Otherwise the tree decides: a pull request's merge
    commit is a new commit, but when its tree equals the branch head's the two
    are the same bytes to build and test, and the branch head's proof is the
    proof. The tree names every tracked byte, lockfile, and submodule pin, so a
    merge that changed anything has a different tree and is refused. Newest
    run wins; an archived commit Git no longer has cannot be matched.
    """
    exact = find_complete(config, commit)
    if exact is not None:
        return exact
    tree = tree_of(config.root, commit)
    archive = config.path(config.runlog.root) / config.runlog.source_archive_dir
    if tree is None or not archive.is_dir() or archive.is_symlink():
        return None
    best: QualificationEvidence | None = None
    for directory in archive.iterdir():
        if not directory.is_dir() or directory.is_symlink():
            continue
        try:
            tested = SourceCommit(directory.name)
        except ValueError:
            continue
        if tree_of(config.root, tested) != tree:
            continue
        found = find_complete(config, tested)
        if found is not None and (best is None or found.reference.run_id > best.reference.run_id):
            best = found
    return best


def missing(commit: SourceCommit) -> GateError:
    """The one refusal for a release whose commit has no passing local proof."""
    return GateError(
        f"source commit {commit} has no passing local qualification on this machine; "
        f"run `just test {commit}` first. A release needs a complete, successful "
        "`just test` for the commit, or for one with the identical Git tree (the "
        "branch head a pull request merged), because the hosted lane takes hours to "
        "find what the local glow-up and functional lanes find in minutes. Failed or "
        f'interrupted attempts do not count; an approved `just test {commit} force "<reason>"` '
        "retry that passes does."
    )


def require_complete(config: GateConfig, commit: SourceCommit) -> QualificationEvidence:
    found = find_release_proof(authority(config), commit)
    if found is None:
        raise missing(commit)
    return found


class AcceptQualification(Action, name="accept-exact-qualification"):
    """Revalidate the exact journal at the graph edge before publication."""

    def __init__(self, commit: SourceCommit) -> None:
        self._commit = commit

    def render(self) -> str:
        return f"require complete qualification journal for {self._commit}"

    def perform(self, context: Context) -> None:
        found = require_complete(context.config, self._commit)
        context.journal.note(
            f"accepted qualification {found.reference.run_id} at {found.reference.run_log} "
            f"({found.reference.digest})"
        )
