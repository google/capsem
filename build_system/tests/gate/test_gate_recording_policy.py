"""What records a run, and what a shared flag does when nothing recorded one.

Two defects with one cause: `records` was a class constant, so it could not
depend on *how* a command was invoked, and `_summarize` assumed the thing
`_recording()` returned always had a run directory.

    $ uv run capsem-gate version --timing
    <the version>
    AttributeError: 'NullJournal' object has no attribute 'directory'

`gc` showed the other half. It is exclusive and reclaims whole trees, and it
was marked `records = False` with the docstring "Only reads runs" copied from
the inspection commands. Only `gc --dry-run` reads. A normal `gc` deletes, and
a partial reclaim left terminal output and no durable evidence -- against the
rule that mutation is locked, visible and timed.
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


def test_a_dry_run_gc_records_nothing() -> None:
    """Asking what would be reclaimed is inspection."""
    command = GateCommand.registry["gc"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=True, graph=False, timing=False, aggressive=False),
    )

    assert not command.should_record()


def test_a_real_gc_records_what_it_deleted() -> None:
    """Deleting whole trees is not inspection, whatever the docstring said."""
    command = GateCommand.registry["gc"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, aggressive=False),
    )

    assert command.should_record(), (
        "a normal gc reclaims disk; a partial one must leave evidence"
    )


def test_every_mutating_command_records() -> None:
    """The guard is the semantic claim, not a list of names.

    A fixed allowlist approved `gc`'s silence by naming it, which is how a
    destructive command came to be classified with the run readers.
    """
    silent = sorted(
        name
        for name, cls in GateCommand.registry.items()
        if cls.__module__.startswith("capsem_builder.gate.")
        and not _command_records(name, cls)
    )

    assert set(silent) <= INSPECTION | {"gc"}, (
        f"{sorted(set(silent) - INSPECTION - {'gc'})} change the machine "
        "without recording it"
    )


def _command_records(name: str, cls) -> bool:
    args = {"gc": {"aggressive": False}}.get(name, {})
    try:
        return cls(
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
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
