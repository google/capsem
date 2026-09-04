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

from . import digestreport, runledger
from .config import GateConfig
from .filesystem import copy_tree, make_dir, remove
from .runhistory import finished, history_locked, point_latest, runs


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

    if imported:
        runledger.sync(host_config, host_config.runlog)
        digestreport.write(host_config)
    return tuple(imported)
