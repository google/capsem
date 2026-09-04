"""What records a run, and what a shared flag does when nothing recorded one.

`_summarize` once assumed the thing `_recording()` returned always had a run
directory.

    $ uv run --project build_system --frozen capsem-gate version --timing
    <the version>
    AttributeError: 'NullJournal' object has no attribute 'directory'

Read-only commands deliberately record nothing. Every mutating command must
remain locked, visible, and timed.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import pytest
from capsem_builder.gate import cli  # noqa: F401 - importing registers every command
from capsem_builder.gate.command import GateCommand
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]

#: Commands that answer a question about runs, and must not create one.
INSPECTION = {"runs", "version"}


def _command(name: str, **args):
    return GateCommand.registry[name](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
    )


@pytest.mark.parametrize("name", sorted(INSPECTION))
def test_inspection_records_nothing(name) -> None:
    assert not _command(name).should_record()


def test_every_mutating_command_records() -> None:
    """The guard is the semantic claim, not a list of mutating names."""
    silent = sorted(
        name
        for name, cls in GateCommand.registry.items()
        if cls.__module__.startswith("capsem_builder.gate.") and not _command_records(name, cls)
    )

    assert set(silent) <= INSPECTION, (
        f"{sorted(set(silent) - INSPECTION)} change the machine without recording it"
    )


def _command_records(name: str, cls) -> bool:
    try:
        return cls(
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False),
        ).should_record()
    except TypeError:
        return True  # needs arguments; it is not one of the readers


def test_timing_on_a_non_recording_command_says_so_instead_of_crashing(capsys) -> None:
    command = _command("version")
    command._args.timing = True

    command.execute()

    printed = capsys.readouterr().out
    assert "AttributeError" not in printed
    assert "no run" in printed.lower() or "records no run" in printed.lower()
