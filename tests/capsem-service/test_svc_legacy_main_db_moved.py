"""An index from before the archive format is moved aside, never migrated.

A build with format v4 ledgers refused an installed `main.db` that predated
`archive_state`, and the service never started after an upgrade. The old
index is now renamed beside the new one and the service starts fresh.
"""

from __future__ import annotations

import os
import sqlite3
import subprocess
import time
from contextlib import closing

import pytest
from helpers.service import PROCESS_BINARY, SERVICE_BINARY, make_service_home_run_dirs
from helpers.sign import sign_binary

pytestmark = pytest.mark.integration


def test_a_legacy_index_is_moved_aside_and_the_service_starts() -> None:
    home_dir, run_dir = make_service_home_run_dirs()
    sessions = home_dir / "sessions"
    sessions.mkdir()
    with closing(sqlite3.connect(sessions / "main.db")) as legacy:
        legacy.execute("CREATE TABLE net_events (id INTEGER PRIMARY KEY)")
        legacy.execute("INSERT INTO net_events DEFAULT VALUES")
        legacy.commit()
    sign_binary(PROCESS_BINARY)
    sign_binary(SERVICE_BINARY)
    socket = run_dir / "service.sock"
    env = {
        **os.environ,
        "CAPSEM_HOME": str(home_dir),
        "CAPSEM_RUN_DIR": str(run_dir),
        "HOME": str(home_dir),
        "RUST_LOG": "info",
    }

    service = subprocess.Popen(
        [
            str(SERVICE_BINARY),
            "--uds-path",
            str(socket),
            "--process-binary",
            str(PROCESS_BINARY),
            "--foreground",
            "--parent-pid",
            str(os.getpid()),
        ],
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        deadline = time.monotonic() + 15
        while (
            not socket.exists()
            and service.poll() is None
            and time.monotonic() < deadline
        ):
            time.sleep(0.05)
        assert service.poll() is None, "the service stopped instead of starting"
        assert socket.exists(), "the service never listened"
    finally:
        service.terminate()
        service.wait(timeout=30)

    retired = [
        path for path in sessions.glob("main.db.retired-*") if "-" not in path.name[-4:]
    ]
    assert len(retired) == 1, sorted(sessions.iterdir())
    with closing(sqlite3.connect(retired[0])) as kept:
        assert kept.execute("SELECT COUNT(*) FROM net_events").fetchone() == (1,)
    logs = "".join(path.read_text() for path in run_dir.glob("service*.log"))
    assert "moved it aside" in logs
