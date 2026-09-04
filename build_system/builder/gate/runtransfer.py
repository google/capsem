"""Bring private run evidence into the host-owned history collection.

A prefix writes its journal in isolation, but the ledger and digest describe
all runs on the machine. Treating that directory like an ordinary exported
tree copied the private ledger over the host ledger and carried the child's
unlocked ``.active`` marker with it. The newest successful run then appeared
older than the failure before it.

Run directories are immutable after ``run.end``. Import only those, strip the
process-local marker, then let the ledger's typed reconciliation ingest them
in chronological order. The aggregate files never cross the prefix boundary.
"""

from __future__ import annotations

from pathlib import Path

from . import auditfs, digestreport, runledger
from .config import GateConfig
from .errors import GateError
from .filesystem import copy_tree, digest_of, make_dir, remove
from .runhistory import finished, history_locked, point_latest, runs


def _export_source_archives(source: GateConfig, target: GateConfig) -> int:
    """Merge immutable exact-source journals into host-owned evidence."""
    source_root = source.path(source.runlog.root) / source.runlog.source_archive_dir
    target_root = target.path(target.runlog.root) / target.runlog.source_archive_dir
    if not source_root.exists():
        return 0
    if source_root.is_symlink() or target_root.is_symlink():
        raise GateError("qualification journal archive roots must not be symlinks")

    imported = 0
    for source_commit in sorted(source_root.iterdir()):
        if source_commit.is_symlink() or not source_commit.is_dir():
            raise GateError(f"invalid qualification archive directory: {source_commit}")
        target_commit = target_root / source_commit.name
        if target_commit.is_symlink():
            raise GateError(f"qualification archive directory is a symlink: {target_commit}")
        make_dir(target_commit)
        for journal in sorted(source_commit.glob("*.jsonl")):
            if journal.is_symlink() or not journal.is_file():
                raise GateError(f"invalid qualification journal: {journal}")
            destination = target_commit / journal.name
            if destination.exists():
                if destination.is_symlink() or not destination.is_file():
                    raise GateError(f"invalid host qualification journal: {destination}")
                source_digest = digest_of(journal, algorithm=source.runlog.artifact_digest)
                target_digest = digest_of(destination, algorithm=target.runlog.artifact_digest)
                if source_digest != target_digest:
                    raise GateError(f"qualification journal collision at {destination}")
                continue
            auditfs.stage(journal, destination)
            imported += 1
    return imported


def export(prefix: Path, destination: Path, config: GateConfig) -> tuple[Path, ...]:
    """Import completed prefix runs and refresh host aggregate evidence."""
    source_config = config.model_copy(update={"root": prefix})
    host_config = config.model_copy(update={"root": destination})
    target_root = host_config.path(host_config.runlog.root)
    imported: list[Path] = []

    with history_locked(host_config):
        make_dir(target_root)
        for source in reversed(runs(source_config)):
            if not finished(source, source_config.runlog):
                continue
            target = target_root / source.name
            if target.exists():
                continue
            copy_tree(source, target)
            remove(target / host_config.runlog.active_marker)
            imported.append(target)
        if imported:
            point_latest(imported[-1], host_config.runlog)
        archived = _export_source_archives(source_config, host_config)

    if imported or archived:
        runledger.sync(host_config, host_config.runlog)
        digestreport.write(host_config)
    return tuple(imported)
