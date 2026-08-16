"""The one gate path that cannot come from config must still track it.

`cli.STARTUP_RECORD` records invocations that die before a run directory
exists, including ones that die *because* `config/gate.toml` could not be read.
It therefore cannot read its location from that file, which makes it the single
exception to "every value lives in `config/gate.toml`".

An exception that nothing checks is just a hardcoded path. If `[runlog].root`
moves, startup failures would keep being written where nobody looks for them,
which is the same invisibility the record exists to end.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

from capsem.gate import cli

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_the_startup_record_lives_under_the_configured_run_log_root() -> None:
    configured = tomllib.loads((PROJECT_ROOT / "config/gate.toml").read_text(encoding="utf-8"))[
        "runlog"
    ]["root"]

    assert cli.STARTUP_RECORD.parent == Path(configured), (
        f"startup failures are written to {cli.STARTUP_RECORD}, but the run log "
        f"root is {configured}; a reader of one would never find the other"
    )


def test_the_startup_record_is_a_sibling_of_the_run_directories() -> None:
    """A file, not a directory: there is no run to make a directory for."""
    assert cli.STARTUP_RECORD.suffix == ".jsonl"
    assert not cli.STARTUP_RECORD.is_absolute()
