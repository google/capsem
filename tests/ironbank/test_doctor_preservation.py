"""A real failed CLI doctor retains its stopped VM's local evidence."""

from __future__ import annotations

import os
import subprocess

import pytest
from helpers.constants import BIN_DIR
from helpers.mock_server import mock_server_lock_directory
from helpers.service import ServiceInstance
from helpers.session_ledger import open_session_ledger
from log_streams import read_log_stream

pytestmark = pytest.mark.integration


def test_failed_cli_doctor_preserves_stopped_vm_evidence():
    service = ServiceInstance(isolated_corp_config=True)
    (service.home_dir / "corp.toml").write_text('''[corp.rules.deny_doctor_fixture]
name = "deny_doctor_fixture"
action = "block"
priority = -100
match = 'process.command.contains("capsem-doctor")'
''')
    try:
        service.start()
        result = subprocess.run(
            [str(BIN_DIR / "capsem"), "--uds-path", str(service.uds_path), "doctor"],
            env={**os.environ, "CAPSEM_HOME": str(service.home_dir),
                 "CAPSEM_RUN_DIR": str(service.tmp_dir),
                 "CAPSEM_MOCK_SERVER_LOCK_DIR": str(mock_server_lock_directory())},
            capture_output=True, text=True, timeout=90, check=False,
        )
        assert result.returncode == 1, (result.stdout, result.stderr)
        assert "Doctor execution" in result.stderr, result.stderr
        assert "cleanup failed" not in result.stderr, result.stderr
        assert service.client().get("/vms/list")["sandboxes"] == []
        preserved = list((service.tmp_dir / "sessions").glob("*-failed-*"))
        assert len(preserved) == 1, (preserved, result.stdout, result.stderr)
        assert list((service.tmp_dir / "sessions").iterdir()) == preserved
        with open_session_ledger(preserved[0] / "session.db") as conn:
            rows = conn.execute("SELECT event_id,rule_action FROM security_rule_events WHERE rule_id='corp.rules.deny_doctor_fixture'").fetchall()
            assert len(rows) == 1 and rows[0][1] == "block"
            event_id = rows[0][0]
            assert conn.execute("SELECT command,exit_code,source FROM exec_events WHERE event_id=?", (event_id,)).fetchall() == [
                ("capsem-doctor --durations=10", None, "api"),
            ]
            decisions = conn.execute("SELECT COALESCE(e.effective_decision,r.effective_decision) FROM security_decision_events e LEFT JOIN security_decision_runs r ON r.id=e.run_id WHERE e.event_id=? ORDER BY e.id", (event_id,)).fetchall()
            assert decisions and decisions[-1][0] == "block"
        logs = service.stop_and_read_log()
        assert "panic" not in logs.lower()
        process_logs = list(preserved[0].glob("**/process.log"))
        assert process_logs and read_log_stream(process_logs[0])
    finally:
        service.stop()
