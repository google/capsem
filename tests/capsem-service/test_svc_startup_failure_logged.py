"""A service that fails to start says why in its own log.

The gate and the installer launch capsem-service detached and, when it dies
before listening, point at its log. A startup error returned from
`run_service` went only to stderr, which nobody reads: a dev build refusing
an installed ledger left a service log that ended at "sqlite mmap telemetry
recorded" with no cause anywhere.
"""

from __future__ import annotations

import os
import sqlite3
import subprocess
from contextlib import closing

import pytest
from helpers.service import PROCESS_BINARY, SERVICE_BINARY, make_service_home_run_dirs
from helpers.sign import sign_binary

pytestmark = pytest.mark.integration


def test_a_startup_error_is_written_to_the_service_log() -> None:
    home_dir, run_dir = make_service_home_run_dirs()
    sessions = home_dir / "sessions"
    sessions.mkdir()
    # A ledger in the current format whose archive identity is missing: the
    # service must refuse it, not move it aside or start on it.
    with closing(sqlite3.connect(sessions / "main.db")) as broken:
        broken.execute("CREATE TABLE archive_state (id INTEGER PRIMARY KEY)")
        broken.commit()
    sign_binary(PROCESS_BINARY)
    sign_binary(SERVICE_BINARY)
    env = {
        **os.environ,
        "CAPSEM_HOME": str(home_dir),
        "CAPSEM_RUN_DIR": str(run_dir),
        "HOME": str(home_dir),
        "RUST_LOG": "info",
    }

    result = subprocess.run(
        [
            str(SERVICE_BINARY),
            "--uds-path",
            str(run_dir / "service.sock"),
            "--process-binary",
            str(PROCESS_BINARY),
            "--foreground",
        ],
        env=env,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )

    assert result.returncode != 0
    logs = sorted(run_dir.glob("service*.log"))
    assert logs, f"no service log in {run_dir}"
    text = "".join(path.read_text() for path in logs)
    assert "archive_state must contain exactly one row" in text, (
        "the cause of a failed start must reach the service log, not only stderr"
    )
