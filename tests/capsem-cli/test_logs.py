"""CLI log retrieval for live and preserved failed sessions."""

import uuid

import pytest

pytestmark = pytest.mark.integration


def test_logs_reads_preserved_failed_session_by_original_id(service_env, cli_runner):
    session_id = str(uuid.uuid4())
    failed_dir = service_env.tmp_dir / "sessions" / f"{session_id}-failed-regression"
    failed_dir.mkdir(parents=True)
    (failed_dir / "process.log").write_text("vhost-vsock permission denied\n")

    stdout, stderr, returncode = cli_runner(
        "logs", session_id, uds_path=service_env.uds_path
    )

    assert returncode == 0, stderr
    assert "vhost-vsock permission denied" in stdout
