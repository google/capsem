"""Ironbank black-box file, process, and snapshot ledger tests."""

from __future__ import annotations

import json
import re
import sqlite3
import textwrap
import time
import uuid
from pathlib import Path

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, vm_session_db_path, vm_session_dir, wait_exec_ready, vm_name


PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"

pytestmark = pytest.mark.integration

EXPECTED_FS_COLUMNS = {
    "id",
    "event_id",
    "timestamp",
    "action",
    "path",
    "directory",
    "name",
    "size",
    "trace_id",
    "turn_id",
    "credential_ref",
}

EXPECTED_EXEC_COLUMNS = {
    "id",
    "event_id",
    "timestamp",
    "exec_id",
    "command",
    "exit_code",
    "duration_ms",
    "stdout_preview",
    "stderr_preview",
    "stdout_bytes",
    "stderr_bytes",
    "source",
    "trace_id",
    "process_name",
    "pid",
    "turn_id",
    "credential_ref",
}

EXPECTED_AUDIT_COLUMNS = {
    "id",
    "event_id",
    "timestamp",
    "pid",
    "ppid",
    "uid",
    "exe",
    "comm",
    "argv",
    "cwd",
    "exit_code",
    "session_id",
    "tty",
    "audit_id",
    "exec_event_id",
    "parent_exe",
    "trace_id",
    "turn_id",
    "credential_ref",
}

SECURITY_ROUTE_FIELDS = {
    "timestamp_unix_ms",
    "event_id",
    "event_type",
    "rule_id",
    "rule_action",
    "detection_level",
    "rule_json",
    "event_json",
    "trace_id",
    "turn_id",
    "credential_ref",
}


def _connect_session_db(service: ServiceInstance, session_id: str) -> sqlite3.Connection:
    db_path = vm_session_db_path(service.tmp_dir, service.client(), session_id)
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


def _eventually(fetch, predicate, *, timeout_s: float = 15.0, interval_s: float = 0.25):
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        last = fetch()
        if predicate(last):
            return last
        time.sleep(interval_s)
    assert predicate(last), f"condition not met before timeout; last={last!r}"
    return last


def _assert_ledger_id(value: object) -> None:
    assert isinstance(value, str)
    assert re.fullmatch(r"[0-9a-f]{12}", value), value


def _rows_by_path(conn: sqlite3.Connection, paths: set[str]) -> dict[str, list[sqlite3.Row]]:
    placeholders = ",".join("?" for _ in paths)
    rows = conn.execute(
        f"SELECT * FROM fs_events WHERE path IN ({placeholders}) ORDER BY id",
        tuple(sorted(paths)),
    ).fetchall()
    by_path: dict[str, list[sqlite3.Row]] = {path: [] for path in paths}
    for row in rows:
        by_path[row["path"]].append(row)
    return by_path


def _write_script(nonce: str, create_path: str, modify_path: str, delete_path: str) -> str:
    return textwrap.dedent(
        f"""
        #!/usr/bin/env bash
        set -euo pipefail
        printf '%s\\n' {nonce!r} > /root/{create_path}
        sleep 1
        printf 'base:%s\\n' {nonce!r} > /root/{modify_path}
        sleep 1
        printf 'changed:%s\\n' {nonce!r} >> /root/{modify_path}
        sleep 1
        printf 'delete:%s\\n' {nonce!r} > /root/{delete_path}
        sleep 1
        rm -f /root/{delete_path}
        ln -sfn /etc/passwd /root/ironbank-symlink-escape
        python3 - <<'PY'
        import json
        from pathlib import Path
        paths = {{
            "created": "/root/{create_path}",
            "modified": "/root/{modify_path}",
            "deleted": "/root/{delete_path}",
            "symlink": "/root/ironbank-symlink-escape",
        }}
        print("IRONBANK_FILE_PROCESS=" + json.dumps({{
            "nonce": {nonce!r},
            "paths": paths,
            "created_text": Path(paths["created"]).read_text(encoding="utf-8").strip(),
            "modified_text": Path(paths["modified"]).read_text(encoding="utf-8").strip(),
            "deleted_exists": Path(paths["deleted"]).exists(),
            "symlink_target": str(Path(paths["symlink"]).readlink()),
        }}, sort_keys=True))
        PY
        """
    ).lstrip()


def _extract_json_line(output: str, prefix: str) -> dict:
    for line in output.splitlines():
        if line.startswith(prefix):
            return json.loads(line.removeprefix(prefix))
    raise AssertionError(f"{prefix!r} missing from output:\n{output}")


def _columnar_rows(payload: dict) -> list[dict]:
    assert set(payload) == {"columns", "rows"}
    columns = payload["columns"]
    assert columns == ["timestamp", "layer", "ref", "summary", "status", "duration_ms", "trace_id"]
    return [dict(zip(columns, row, strict=True)) for row in payload["rows"]]


def test_file_process_snapshot_routes_pay_full_ledger_debt_blackbox():
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    session_id = vm_name("ironbank-fps")
    client = None
    nonce = f"fps-{uuid.uuid4().hex}"
    upload_path = f"ironbank-upload-{uuid.uuid4().hex[:8]}.txt"
    create_path = f"ironbank-created-{uuid.uuid4().hex[:8]}.txt"
    modify_path = f"ironbank-modified-{uuid.uuid4().hex[:8]}.txt"
    delete_path = f"ironbank-deleted-{uuid.uuid4().hex[:8]}.txt"
    script_path = f"ironbank-file-process-{uuid.uuid4().hex[:8]}.sh"
    upload_body = f"upload:{nonce}\n".encode()

    try:
        service.start()
        client = service.client()
        create = client.post(
            "/vms/create",
            {
                "name": session_id,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
            },
            timeout=90,
        )
        assert create is not None
        assert create.get("id") == session_id or create.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path={upload_path}",
            upload_body,
            timeout=30,
        )
        assert upload == {"success": True, "size": len(upload_body)}

        read_status, read_body = client.get_bytes(
            f"/vms/{session_id}/files/content?path={upload_path}",
            timeout=30,
        )
        assert read_status == 200
        assert read_body == upload_body

        listing = client.get(f"/vms/{session_id}/files/list?depth=1", timeout=30)
        entries = {entry["path"]: entry for entry in listing["entries"]}
        assert upload_path in entries
        assert entries[upload_path]["name"] == upload_path
        assert entries[upload_path]["type"] == "file"
        assert entries[upload_path]["size"] == len(upload_body)
        assert entries[upload_path]["mime"] == "text/plain"
        assert entries[upload_path]["is_text"] is True

        script = _write_script(nonce, create_path, modify_path, delete_path).encode()
        script_upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path={script_path}",
            script,
            timeout=30,
        )
        assert script_upload == {"success": True, "size": len(script)}

        exec_resp = client.post(
            f"/vms/{session_id}/exec",
            {"command": f"bash /root/{script_path}", "timeout_secs": 90},
            timeout=110,
        )
        assert exec_resp is not None
        assert exec_resp["exit_code"] == 0, exec_resp
        result = _extract_json_line(exec_resp["stdout"], "IRONBANK_FILE_PROCESS=")
        assert result["nonce"] == nonce
        assert result["created_text"] == nonce
        assert result["modified_text"] == f"base:{nonce}\nchanged:{nonce}"
        assert result["deleted_exists"] is False
        assert result["symlink_target"] == "/etc/passwd"

        escape_status, escape_body = client.get_bytes(
            f"/vms/{session_id}/files/content?path=ironbank-symlink-escape",
            timeout=30,
        )
        assert escape_status == 403, escape_body.decode(errors="replace")
        assert b"root:" not in escape_body

        snapshot_status = client.get(f"/vms/{session_id}/snapshots/status", timeout=30)
        assert set(snapshot_status) == {
            "total",
            "auto_count",
            "manual_count",
            "manual_available",
            "snapshots",
        }
        assert isinstance(snapshot_status["snapshots"], list)
        assert snapshot_status["total"] == snapshot_status["auto_count"] + snapshot_status["manual_count"]

        snapshot_list = client.get(f"/vms/{session_id}/snapshots/list", timeout=30)
        assert set(snapshot_list) == {"total", "snapshots"}
        assert snapshot_list["total"] == snapshot_status["total"]
        assert snapshot_list["snapshots"] == snapshot_status["snapshots"]

        conn = _connect_session_db(service, session_id)
        try:
            assert _table_columns(conn, "fs_events") == EXPECTED_FS_COLUMNS
            assert _table_columns(conn, "exec_events") == EXPECTED_EXEC_COLUMNS
            assert _table_columns(conn, "audit_events") == EXPECTED_AUDIT_COLUMNS
            assert not conn.execute(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'snapshot_events'"
            ).fetchone(), "snapshot route state must stay route-owned"

            paths = {upload_path, script_path, create_path, modify_path, delete_path}
            file_rows = _eventually(
                lambda: _rows_by_path(conn, paths),
                lambda rows: (
                    any(row["action"] == "import" for row in rows[upload_path])
                    and any(row["action"] == "export" for row in rows[upload_path])
                    and any(row["action"] in {"created", "modified"} for row in rows[create_path])
                    and any(row["action"] == "modified" for row in rows[modify_path])
                    and any(row["action"] == "deleted" for row in rows[delete_path])
                ),
                timeout_s=25,
            )
            for path, rows in file_rows.items():
                assert rows, f"{path} missing fs_events rows"
                for row in rows:
                    _assert_ledger_id(row["event_id"])
                    assert row["path"] == path
                    assert row["name"] == Path(path).name
                    assert row["directory"] in {".", str(Path(path).parent)}
                    assert row["credential_ref"] is None
                    assert row["size"] is None or row["size"] >= 0
            assert [row["size"] for row in file_rows[upload_path] if row["action"] == "import"][-1] == len(upload_body)
            assert [row["size"] for row in file_rows[upload_path] if row["action"] == "export"][-1] == len(upload_body)

            exec_row = _eventually(
                lambda: conn.execute(
                    "SELECT * FROM exec_events WHERE command = ? ORDER BY id DESC LIMIT 1",
                    (f"bash /root/{script_path}",),
                ).fetchone(),
                lambda row: row is not None and row["exit_code"] == 0,
            )
            _assert_ledger_id(exec_row["event_id"])
            assert exec_row["source"] == "api"
            assert exec_row["stdout_bytes"] >= len("IRONBANK_FILE_PROCESS=")
            assert "IRONBANK_FILE_PROCESS=" in exec_row["stdout_preview"]
            assert exec_row["stderr_preview"] in {None, ""}
            assert exec_row["credential_ref"] is None

            audit_rows = _eventually(
                lambda: conn.execute(
                    "SELECT * FROM audit_events WHERE argv LIKE ? OR exe LIKE ? ORDER BY id",
                    (f"%{script_path}%", "%/bash"),
                ).fetchall(),
                lambda rows: len(rows) >= 1,
                timeout_s=15,
            )
            assert any("bash" in row["exe"] for row in audit_rows)
            for row in audit_rows[:10]:
                _assert_ledger_id(row["event_id"])
                assert row["pid"] > 0
                assert row["ppid"] >= 0
                assert row["uid"] == 0
                assert row["argv"]
                assert row["credential_ref"] is None

            security_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM security_rule_events
                    WHERE event_id IN (
                        SELECT event_id FROM fs_events
                        WHERE path IN (?, ?, ?, ?, ?)
                    )
                    ORDER BY id
                    """,
                    (upload_path, script_path, create_path, modify_path, delete_path),
                ).fetchall(),
                lambda rows: len(rows) >= 4,
                timeout_s=15,
            )
            assert {row["rule_action"] for row in security_rows} == {"allow"}
            assert {row["rule_id"] for row in security_rows} == {"profiles.rules.default_file"}
            assert {row["event_type"] for row in security_rows} >= {
                "file.import",
                "file.export",
                "file.event",
            }
            for row in security_rows:
                _assert_ledger_id(row["event_id"])
                event_json = json.loads(row["event_json"])
                assert event_json["event_type"] in {"file.import", "file.export", "file.event"}
                assert event_json["file"] is not None
                assert event_json["decision"]["effective"] == "allow"
                assert row["detection_level"] == "none"

            timeline = _eventually(
                lambda: client.get(
                    f"/vms/{session_id}/timeline?layers=fs,exec&limit=200",
                    timeout=30,
                ),
                lambda payload: (
                    len(_columnar_rows(payload)) >= len(paths)
                    and {"fs", "exec"}
                    <= {event["layer"] for event in _columnar_rows(payload)}
                ),
            )
            timeline_rows = _columnar_rows(timeline)
            assert len(timeline_rows) >= len(paths)
            layers = {event["layer"] for event in timeline_rows}
            assert {"fs", "exec"} <= layers
            summaries = "\n".join(event["summary"] for event in timeline_rows)
            assert upload_path in summaries
            assert script_path in summaries
            assert "snapshot" not in summaries.lower()

            history = client.get(f"/vms/{session_id}/history?layer=exec&limit=20", timeout=30)
            assert history["total"] >= 1
            assert any(
                entry["layer"] == "exec"
                and entry["command"] == f"bash /root/{script_path}"
                and entry["exit_code"] == 0
                and entry["details"]["source"] == "api"
                for entry in history["commands"]
            )

            counts = client.get(f"/vms/{session_id}/history/counts", timeout=30)
            assert counts["exec_count"] >= 1
            assert counts["audit_count"] >= 1

            processes = client.get(f"/vms/{session_id}/history/processes", timeout=30)
            assert set(processes) == {"processes"}
            assert any(proc["exe"].endswith("/bash") for proc in processes["processes"])
            for proc in processes["processes"][:10]:
                assert set(proc) == {"exe", "command_count", "first_seen", "last_seen"}
                assert proc["command_count"] >= 1
                assert proc["first_seen"] <= proc["last_seen"]

            security_latest = client.get(f"/vms/{session_id}/security/latest?limit=200", timeout=30)
            assert isinstance(security_latest, list)
            assert security_latest
            for item in security_latest[:10]:
                assert set(item) == SECURITY_ROUTE_FIELDS
            latest_file_events = [
                item
                for item in security_latest
                if item["event_type"] in {"file.import", "file.export", "file.event"}
            ]
            assert latest_file_events
            assert any(
                json.loads(item["event_json"])["file"].get("import_name") == upload_path
                for item in latest_file_events
            )
        finally:
            conn.close()

        process_log = (vm_session_dir(service.tmp_dir, client, session_id) / "process.log").read_text(
            encoding="utf-8",
            errors="replace",
        )
        assert "fs-monitor" in process_log
        assert "snapshot_events" not in process_log
    finally:
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        service.stop()
