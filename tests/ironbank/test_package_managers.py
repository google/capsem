"""Ironbank black-box package-manager ledger tests.

These tests intentionally drive Capsem through public service routes and the
guest VM. They do not use product internals to decide what should happen.
"""

import sqlite3
import textwrap
import time
import uuid

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready, vm_name

pytestmark = pytest.mark.integration

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
    "mcp_call_id",
    "trace_id",
    "process_name",
    "pid",
    "credential_ref",
}

EXPECTED_FS_COLUMNS = {
    "id",
    "event_id",
    "timestamp",
    "action",
    "path",
    "size",
    "trace_id",
    "credential_ref",
}


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


def _connect_session_db(service: ServiceInstance, session_id: str) -> sqlite3.Connection:
    db_path = service.tmp_dir / "sessions" / session_id / "session.db"
    assert db_path.exists(), f"session.db missing at {db_path}"
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def _eventually(fetch, predicate, *, timeout_s: float = 10.0, interval_s: float = 0.25):
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        last = fetch()
        if predicate(last):
            return last
        time.sleep(interval_s)
    assert predicate(last), f"condition not met before timeout; last={last!r}"
    return last


def _package_probe_script() -> str:
    return textwrap.dedent(
        r'''
        #!/usr/bin/env bash
        set -euo pipefail

        work="/root/ironbank-package-probe"
        rm -rf "$work"
        mkdir -p "$work"/{wheels,npm/bin,deb/DEBIAN,deb/usr/local/bin}
        printf 'ironbank-package-bytes\n' > "$work/payload.txt"

        node - <<'JS'
        const fs = require("fs");
        const value = fs.readFileSync("/root/ironbank-package-probe/payload.txt", "utf8").trim();
        console.log("IRONBANK:node:" + value.toUpperCase());
        JS

        python3 - <<'PY'
        import textwrap
        import zipfile
        from pathlib import Path

        root = Path("/root/ironbank-package-probe/wheels")

        def wheel(distribution, module, source):
            version = "0.1.0"
            normalized = distribution.replace("-", "_")
            dist_info = f"{normalized}-{version}.dist-info"
            wheel_path = root / f"{normalized}-{version}-py3-none-any.whl"
            files = {
                f"{module}/__init__.py": textwrap.dedent(source).lstrip(),
                f"{dist_info}/METADATA": (
                    "Metadata-Version: 2.1\n"
                    f"Name: {distribution}\n"
                    f"Version: {version}\n"
                ),
                f"{dist_info}/WHEEL": (
                    "Wheel-Version: 1.0\n"
                    "Generator: ironbank\n"
                    "Root-Is-Purelib: true\n"
                    "Tag: py3-none-any\n"
                ),
            }
            record = [f"{name},," for name in files]
            record.append(f"{dist_info}/RECORD,,")
            files[f"{dist_info}/RECORD"] = "\n".join(record) + "\n"
            with zipfile.ZipFile(wheel_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
                for name, data in files.items():
                    zf.writestr(name, data)
            return wheel_path

        wheel(
            "ironbank-pip-pkg",
            "ironbank_pip_pkg",
            """
            def answer():
                return 42
            """,
        )
        wheel(
            "ironbank-uv-pkg",
            "ironbank_uv_pkg",
            """
            def marker():
                return "uv:ironbank"
            """,
        )
        PY

        pip install --no-index "$work/wheels/ironbank_pip_pkg-0.1.0-py3-none-any.whl" >/tmp/ironbank-pip.log 2>&1
        python3 - <<'PY'
        import ironbank_pip_pkg
        print(f"IRONBANK:pip:{ironbank_pip_pkg.answer()}")
        PY

        uv pip install --python /root/.venv/bin/python --no-index "$work/wheels/ironbank_uv_pkg-0.1.0-py3-none-any.whl" >/tmp/ironbank-uv.log 2>&1
        /root/.venv/bin/python - <<'PY'
        import ironbank_uv_pkg
        print(f"IRONBANK:uv:{ironbank_uv_pkg.marker()}")
        PY

        cat > "$work/npm/package.json" <<'JSON'
        {"name":"ironbank-npm-pkg","version":"0.1.0","bin":{"ironbank-npm-pkg":"bin/cli.js"}}
        JSON
        cat > "$work/npm/bin/cli.js" <<'JS'
        #!/usr/bin/env node
        console.log("IRONBANK:npm:npm:realm")
        JS
        chmod 755 "$work/npm/bin/cli.js"
        npm install -g "file:$work/npm" >/tmp/ironbank-npm.log 2>&1
        ironbank-npm-pkg
        printf 'IRONBANK:npx:'
        npx --yes --package "file:$work/npm" ironbank-npm-pkg | sed 's/^IRONBANK:npm://'

        cat > "$work/deb/DEBIAN/control" <<'EOF'
        Package: ironbank-apt-tool
        Version: 0.1.0
        Section: utils
        Priority: optional
        Architecture: all
        Maintainer: Capsem Ironbank <ironbank@capsem.local>
        Description: Hermetic apt package-manager probe
        EOF
        cat > "$work/deb/usr/local/bin/ironbank-apt-tool" <<'SH'
        #!/bin/sh
        printf 'IRONBANK:apt:apt:'
        tr '[:upper:]' '[:lower:]' < "$1" | tr -d '\n'
        printf '\n'
        SH
        chmod 755 "$work/deb/usr/local/bin/ironbank-apt-tool"
        dpkg-deb --build "$work/deb" "$work/ironbank-apt-tool.deb" >/tmp/ironbank-dpkg.log 2>&1
        apt-get install -y -qq "$work/ironbank-apt-tool.deb" >/tmp/ironbank-apt.log 2>&1
        ironbank-apt-tool "$work/payload.txt"

        printf 'IRONBANK:complete:apt+npm+npx+node+pip+uv\n'
        '''
    ).lstrip()


def test_package_managers_pay_their_ledger_debt_blackbox():
    service = ServiceInstance()
    session_id = vm_name("ironbank-pkg")
    script_name = f"ironbank-package-probe-{uuid.uuid4().hex[:8]}.sh"
    client = None
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
        assert create is not None, "session creation returned no body"
        assert create.get("id") == session_id or create.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

        script_bytes = _package_probe_script().encode()
        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path={script_name}",
            script_bytes,
            timeout=30,
        )
        assert upload is not None, "script upload returned no body"
        assert upload.get("success") is True, f"script upload failed: {upload}"
        assert upload.get("size") == len(script_bytes), f"uploaded script size mismatch: {upload}"

        status_before = client.get(f"/vms/{session_id}/status", timeout=30)
        assert status_before is not None
        assert status_before.get("id") == session_id or status_before.get("name") == session_id
        assert isinstance(status_before.get("available_actions"), list)

        exec_resp = client.post(
            f"/vms/{session_id}/exec",
            {"command": f"bash /root/{script_name}", "timeout_secs": 240},
            timeout=260,
        )
        assert exec_resp is not None, "exec returned no body"
        assert exec_resp.get("exit_code") == 0, exec_resp
        stdout = exec_resp.get("stdout", "")
        stderr = exec_resp.get("stderr", "")
        assert "IRONBANK:node:IRONBANK-PACKAGE-BYTES" in stdout
        assert "IRONBANK:pip:42" in stdout
        assert "IRONBANK:uv:uv:ironbank" in stdout
        assert "IRONBANK:npm:npm:realm" in stdout
        assert "IRONBANK:npx:npm:realm" in stdout
        assert "IRONBANK:apt:apt:ironbank-package-bytes" in stdout
        assert "IRONBANK:complete:apt+npm+npx+node+pip+uv" in stdout
        assert "No space left on device" not in stdout + stderr
        assert "Permission denied" not in stdout + stderr
        assert "externally-managed" not in (stdout + stderr).lower()

        history = client.get(f"/vms/{session_id}/history", timeout=30)
        assert history is not None
        assert history.get("total", 0) >= 1
        history_text = " ".join(
            (entry.get("command") or "") + " " + (entry.get("stdout_preview") or "")
            for entry in history.get("commands", [])
        )
        assert script_name in history_text
        assert "IRONBANK:complete" in history_text

        counts = client.get(f"/vms/{session_id}/history/counts", timeout=30)
        assert counts is not None
        assert isinstance(counts.get("exec_count"), int) and counts["exec_count"] >= 1
        assert isinstance(counts.get("audit_count"), int) and counts["audit_count"] >= 0

        conn = _connect_session_db(service, session_id)
        try:
            assert _table_columns(conn, "exec_events") == EXPECTED_EXEC_COLUMNS
            assert _table_columns(conn, "fs_events") == EXPECTED_FS_COLUMNS

            exec_row = _eventually(
                lambda: conn.execute(
                    "SELECT * FROM exec_events WHERE command LIKE ? ORDER BY id DESC LIMIT 1",
                    (f"%{script_name}%",),
                ).fetchone(),
                lambda row: row is not None and row["exit_code"] == 0,
                timeout_s=15,
            )
            assert exec_row["command"] == f"bash /root/{script_name}"
            assert isinstance(exec_row["event_id"], str) and len(exec_row["event_id"]) == 12
            assert exec_row["source"] == "api"
            assert exec_row["exit_code"] == 0
            assert exec_row["duration_ms"] >= 0
            assert exec_row["stdout_bytes"] >= len("IRONBANK:complete")
            assert exec_row["stderr_bytes"] >= 0
            assert "IRONBANK:complete" in (exec_row["stdout_preview"] or "")
            assert "No space left" not in (exec_row["stderr_preview"] or "")
            assert exec_row["credential_ref"] is None

            fs_rows = _eventually(
                lambda: conn.execute(
                    "SELECT * FROM fs_events WHERE path LIKE ? ORDER BY id",
                    (f"%{script_name}%",),
                ).fetchall(),
                lambda rows: len(rows) >= 1,
                timeout_s=15,
            )
            assert any(row["action"] in {"created", "modified"} for row in fs_rows)
            assert all(isinstance(row["event_id"], str) and len(row["event_id"]) == 12 for row in fs_rows)
            assert all(row["path"] for row in fs_rows)
            assert all(row["size"] is None or row["size"] >= 0 for row in fs_rows)
            assert all(row["credential_ref"] is None for row in fs_rows)
        finally:
            conn.close()
    finally:
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        service.stop()
