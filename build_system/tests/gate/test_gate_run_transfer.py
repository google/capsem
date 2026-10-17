"""Private journals join host history without replacing its aggregates."""

from __future__ import annotations

import json
import os
import sys
from contextlib import nullcontext
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import prefix as gate_prefix
from capsem_builder.gate import runledger, runtransfer
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.funnel import GuardedRunner
from capsem_builder.gate.invocation import ConsoleMode
from capsem_builder.gate.proc import Runner
from capsem_builder.gate.processgroup import StopPolicy
from capsem_builder.gate.runhistory import read
from capsem_builder.gate.runlog import RunLog

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


def _reclaim_config(tmp_path: Path) -> gate_config.GateConfig:
    host, parent = tmp_path / "host", tmp_path / "prefixes"
    host.mkdir()
    return BASE.model_copy(
        update={
            "root": host,
            "prefix": BASE.prefix.model_copy(
                update={
                    "parent": str(parent),
                    "build_cache": str(tmp_path / "products"),
                }
            ),
        }
    )


@pytest.mark.parametrize("failed", (False, True))
def test_reclaim_preserves_a_finished_run_abandoned_by_its_caller(
    tmp_path: Path, failed: bool
) -> None:
    """A later sweep must not delete diagnostics the caller never exported."""
    config = _reclaim_config(tmp_path)
    private = gate_prefix.parent_dir(config) / "abandoned"
    private.mkdir(parents=True)
    private_config = config.model_copy(update={"root": private})
    script = (
        "import sys; print('RESOURCE-OUT'); sys.stdout.flush(); "
        f"print('RESOURCE-ERR', file=sys.stderr); sys.exit({9 if failed else 0})"
    )
    with (
        pytest.raises(GateError) if failed else nullcontext(),
        RunLog.open(private_config, "image-qualify") as log,
    ):
        log.shape((), ())
        runner = GuardedRunner.sized_by(
            Runner(private, stop_policy=StopPolicy.from_execution(BASE.execution)),
            private_config,
            journal=log,
        )
        runner.run([sys.executable, "-c", script], console=ConsoleMode.LOG_ONLY)
    original = (log.directory / config.runlog.events).read_bytes()
    resource = log.step_output().read_bytes()

    gate_prefix.reclaim(config, private)

    assert not private.exists()
    retained = config.root / config.runlog.root / log.directory.name
    assert (retained / config.runlog.events).read_bytes() == original
    output = retained / config.runlog.step_log_dir / config.runlog.resource_log
    assert output.read_bytes() == resource
    events = read(retained, config.runlog)
    end = next(event for event in events if event["event"] == "run.end")
    assert end["status"] == ("failed" if failed else "ok")
    executed = next(event for event in events if event["event"] == "exec")
    assert executed["exit"] == (9 if failed else 0)
    span = executed["output"]
    assert span["file"] == output.name
    assert resource[span["offset"] : span["offset"] + span["length"]] == (
        b"RESOURCE-OUT\nRESOURCE-ERR\n"
    )
    assert not (retained / config.runlog.active_marker).exists()
    assert [row.status for row in runledger.rows(config)] == [end["status"]]


@pytest.mark.parametrize("terminal", ("", '{"event": "run.end"}\n'))
def test_reclaim_cannot_turn_unfinished_history_into_completed_evidence(
    tmp_path: Path, terminal: str
) -> None:
    config = _reclaim_config(tmp_path)
    private = gate_prefix.parent_dir(config) / "abandoned"
    directory = _record(private, "20260101-000000-aaaaaa-focus-test", 10.0)
    journal = directory / config.runlog.events
    lines = journal.read_text(encoding="utf-8").splitlines(keepends=True)
    journal.write_text("".join(lines[:-1]) + terminal, encoding="utf-8")

    gate_prefix.reclaim(config, private)

    assert not private.exists()
    assert runledger.rows(config) == [], "no complete run is proved by missing terminal fields"


def test_reclaim_refuses_a_link_before_importing_foreign_history(tmp_path: Path) -> None:
    config = _reclaim_config(tmp_path)
    parent = gate_prefix.parent_dir(config)
    parent.mkdir(parents=True)
    outside = tmp_path / "foreign"
    journal = _record(outside, "20260101-000000-aaaaaa-focus-test", 10.0)
    original = (journal / config.runlog.events).read_bytes()
    link = parent / "alias"
    link.symlink_to(outside, target_is_directory=True)

    with pytest.raises(GateError):
        gate_prefix.reclaim(config, link)

    assert link.is_symlink()
    assert (journal / config.runlog.events).read_bytes() == original
    assert not (config.root / config.runlog.root).exists(), "a refusal imports no foreign bytes"
