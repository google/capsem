"""Two live runs cannot delete or repoint one another.

Run ids are collision-resistant, so two processes never pick the same
directory. Allocation and retention were still uncoordinated: each opened and
rotated its log *before* taking the machine lock, `rotate()` protected only its
own caller's directory, and `latest` was unlinked and recreated with no
coordination at all.

Under a tight retention cap that is enough for one waiting process to classify
another process's live run as unfinished and delete it -- which is the run
somebody is about to want, since it is the one still going.

The fix is deliberately *not* the machine lock: that is held for the length of
a gate, and allocation must not wait forty minutes. A short-lived flock around
allocate/rotate/point-latest is enough, because those are the only operations
that touch another run's directory.
"""

from __future__ import annotations

import json
import multiprocessing
from pathlib import Path

from helpers.runlog_worker import open_and_hold

from capsem.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _checkout(tmp_path: Path, *, keep_runs: int) -> Path:
    """A throwaway checkout whose retention keeps almost nothing."""
    (tmp_path / "config").mkdir(parents=True, exist_ok=True)
    source = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    source = source.replace("keep_runs = 20", f"keep_runs = {keep_runs}")
    (tmp_path / "config" / "gate.toml").write_text(source, encoding="utf-8")
    (tmp_path / "justfile").write_text("# a checkout needs one\n", encoding="utf-8")
    return tmp_path


def test_a_live_run_is_never_rotated_away_by_another(tmp_path: Path) -> None:
    """Retention keeps one, and two runs are already going.

    A third opening run has to drop something. Every candidate is unfinished,
    because unfinished is what a running gate looks like -- so the oldest live
    run is exactly what the old ordering reached for first.
    """
    root = _checkout(tmp_path, keep_runs=1)
    context = multiprocessing.get_context("spawn")
    ready, go = context.Queue(), context.Queue()

    live = []
    for name in ("candidate", "smoke"):
        worker = context.Process(target=open_and_hold, args=(str(root), name, ready, go))
        worker.start()
        live.append((worker, ready.get(timeout=60)))

    settings = gate_config.load(root)
    directory = root / settings.runlog.root
    try:
        # The third one rotates while both of those are still being written.
        third = context.Process(target=open_and_hold, args=(str(root), "lint", ready, go))
        third.start()
        newest = ready.get(timeout=60)
        live.append((third, newest))

        for _worker, name in live[:2]:
            assert (directory / name).is_dir(), (
                f"{name} was still being written and was rotated away by a "
                "run that started after it"
            )
    finally:
        for _worker, _name in live:
            go.put(True)
        for worker, _name in live:
            worker.join(timeout=60)

    # And each closed cleanly, which means none lost its events.
    for _worker, name in live:
        events = (directory / name / settings.runlog.events).read_text(encoding="utf-8")
        kinds = [json.loads(line)["event"] for line in events.splitlines() if line]
        assert "run.start" in kinds and "run.end" in kinds, name


def test_the_history_lock_is_not_the_machine_lock(tmp_path: Path) -> None:
    """Allocation must not wait out a forty-minute gate to get a directory."""
    from capsem.gate import runhistory

    settings = gate_config.load(_checkout(tmp_path, keep_runs=5))

    assert runhistory.history_lock_path(settings) != Path(settings.locks.gate.path), (
        "sharing the gate lock would make opening a run log wait for the run "
        "that is already holding it"
    )
