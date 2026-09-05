"""Private journals join host history without replacing its aggregates."""

from __future__ import annotations

import json
import os
from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.gate import runledger, runtransfer

PROJECT_ROOT = Path(__file__).resolve().parents[3]
BASE = gate_config.load(PROJECT_ROOT)


def _record(root: Path, run_id: str, duration_ms: float) -> Path:
    directory = root / BASE.runlog.root / run_id
    directory.mkdir(parents=True)
    events = [
        {
            "event": "run.start",
            "run_id": run_id,
            "command": "focus-test",
            "argv": ["focus-test"],
            "head": "0" * 40,
            "platform": "Linux",
            "machine": "x86_64",
            "cores": 8,
            "free_gb": 100.0,
            "gate_source": "src",
            "pycache": "cache",
        },
        {"event": "plan", "run_id": run_id, "steps": ["build"], "edges": []},
        {
            "event": "step.end",
            "run_id": run_id,
            "step": "build",
            "duration_ms": duration_ms,
            "status": "ok",
            "error": None,
        },
        {
            "event": "run.end",
            "run_id": run_id,
            "status": "ok",
            "duration_ms": duration_ms,
            "failures": {},
        },
    ]
    (directory / BASE.runlog.events).write_text(
        "\n".join(json.dumps(event) for event in events) + "\n", encoding="utf-8"
    )
    return directory


def test_export_imports_journal_but_reconciles_host_ledger(tmp_path: Path) -> None:
    host, prefix = tmp_path / "host", tmp_path / "prefix"
    config = BASE.model_copy(update={"root": host})
    old_id = "20260101-000000-aaaaaa-focus-test"
    new_id = "20260102-000000-bbbbbb-focus-test"
    old = _record(host, old_id, 10.0)
    runledger.sync(config, config.runlog)
    old_bytes = (old / config.runlog.events).read_bytes()

    new = _record(prefix, new_id, 5.0)
    prefix_config = config.model_copy(update={"root": prefix})
    runledger.sync(prefix_config, prefix_config.runlog)
    (new / config.runlog.active_marker).touch()
    source_archive = (
        prefix
        / config.runlog.root
        / config.runlog.source_archive_dir
        / ("0" * 40)
        / f"{new_id}.jsonl"
    )
    source_archive.parent.mkdir(parents=True)
    os.link(new / config.runlog.events, source_archive)

    assert [path.name for path in runtransfer.export(prefix, host, config)] == [new_id]
    assert [row.run_id for row in runledger.rows(config)] == [new_id, old_id]
    assert (old / config.runlog.events).read_bytes() == old_bytes
    assert not (host / config.runlog.root / new_id / config.runlog.active_marker).exists()
    assert (host / config.runlog.root / config.runlog.latest_link).readlink() == Path(new_id)
    host_archive = (
        host
        / config.runlog.root
        / config.runlog.source_archive_dir
        / ("0" * 40)
        / f"{new_id}.jsonl"
    )
    assert host_archive.read_bytes() == (new / config.runlog.events).read_bytes()


def test_incomplete_terminal_event_is_not_ledger_evidence(tmp_path: Path) -> None:
    config = BASE.model_copy(update={"root": tmp_path})
    directory = tmp_path / config.runlog.root / "20260101-000000-aaaaaa-focus-test"
    directory.mkdir(parents=True)
    (directory / config.runlog.events).write_text('{"event":"run.end"}\n', encoding="utf-8")

    assert runledger.sync(config, config.runlog) == 0
    assert runledger.rows(config) == []
