"""A service that fails to start says why in its own log.

The gate and installer point to the service log when startup fails before
listening. A corrupt host ledger must record its cause there, rather than
leaving it only on the detached process's stderr.
"""

from __future__ import annotations

import os
import subprocess

import pytest
from helpers.service import (
    PROCESS_BINARY,
    SERVICE_BINARY,
    ServiceInstance,
    make_service_home_run_dirs,
)
from helpers.sign import sign_binary
from log_streams import read_log_stream

pytestmark = pytest.mark.integration


def test_a_startup_error_is_written_to_the_service_log() -> None:
    home_dir, run_dir = make_service_home_run_dirs()
    sessions = home_dir / "sessions"
    sessions.mkdir()
    # Pre-v4 ledgers are archived and replaced. A corrupt SQLite file still
    # fails startup, so it exercises the error-log contract directly.
    (sessions / "host.db").write_bytes(b"corrupt host ledger")
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
    assert "file is not a database" in text, (
        "the cause of a failed start must reach the service log, not only stderr"
    )


def test_fixture_preserves_actual_startup_stderr_before_cleanup(tmp_path, monkeypatch, capsys):
    artifacts = tmp_path / "evidence"
    monkeypatch.setenv("CAPSEM_TEST_ARTIFACTS_ROOT", str(artifacts))
    service = ServiceInstance()
    service.gateway_port = -1  # The actual service CLI rejects this before listening.
    try:
        with pytest.raises(RuntimeError, match="failed to accept connections"):
            service.start()
    finally:
        service.stop()
    snapshots = list(artifacts.glob(f"*/{service.home_dir.name}*"))
    assert snapshots, "startup cleanup deleted the only copy of the actual process error"
    error = "unexpected argument '-1' found"
    assert any(error in read_log_stream(path / "run/service.log") for path in snapshots)
    assert error in capsys.readouterr().err
    assert not service.home_dir.exists()
