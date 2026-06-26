"""Ironbank DB rehydration gate.

The logger DB is the ledger. This gate deliberately shells through the public
Cargo test target so release verification cannot forget the Rust-side
flush/restart exactness proof that routes depend on.
"""

from __future__ import annotations

import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def test_logger_db_query_exact_after_flush_and_restart() -> None:
    result = subprocess.run(
        [
            "cargo",
            "test",
            "-p",
            "capsem-logger",
            "db_query_exact_after_flush_and_restart",
            "--",
            "--nocapture",
        ],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )

    assert result.returncode == 0, result.stdout
    assert "db_correctness_db_query_exact_after_flush_and_restart" in result.stdout
    assert "test result: ok. 1 passed" in result.stdout
