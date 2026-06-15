"""Ironbank black-box package-manager ledger tests."""

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
from helpers.service import ServiceInstance, wait_exec_ready, vm_name


PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"

pytestmark = pytest.mark.integration


def _connect_session_db(service: ServiceInstance, session_id: str) -> sqlite3.Connection:
    db_path = service.tmp_dir / "sessions" / session_id / "session.db"
    assert db_path.exists(), f"session.db missing at {db_path}"
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


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


def _columnar_rows(payload: dict) -> list[dict]:
    assert set(payload) == {"columns", "rows"}
    columns = payload["columns"]
    assert columns == ["timestamp", "layer", "ref", "summary", "status", "duration_ms", "trace_id"]
    return [dict(zip(columns, row, strict=True)) for row in payload["rows"]]


def _package_probe_script() -> str:
    return textwrap.dedent(
        r'''
        #!/usr/bin/env bash
        set -euo pipefail

        work="/root/ironbank-package-probe"
        rm -rf "$work"
        mkdir -p "$work"/{wheels,npm/bin,deb/DEBIAN,deb/usr/local/bin,zstd}
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

        if command -v zstd >/dev/null 2>&1; then
          zstd -q -f "$work/payload.txt" -o "$work/zstd/payload.txt.zst"
          zstd -q -d -f "$work/zstd/payload.txt.zst" -o "$work/zstd/payload.roundtrip.txt"
          cmp "$work/payload.txt" "$work/zstd/payload.roundtrip.txt"
          printf 'IRONBANK:zstd:roundtrip\n'
        fi

        printf 'IRONBANK:complete:apt+npm+npx+node+pip+uv\n'
        '''
    ).lstrip()


def test_package_managers_pay_their_ledger_debt_blackbox():
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

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
        assert create is not None
        assert create.get("id") == session_id or create.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

        script_bytes = _package_probe_script().encode()
        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path={script_name}",
            script_bytes,
            timeout=30,
        )
        assert upload == {"success": True, "size": len(script_bytes)}

        exec_resp = client.post(
            f"/vms/{session_id}/exec",
            {"command": f"bash /root/{script_name}", "timeout_secs": 260},
            timeout=290,
        )
        assert exec_resp is not None
        assert exec_resp["exit_code"] == 0, exec_resp
        stdout = exec_resp.get("stdout", "")
        stderr = exec_resp.get("stderr", "")
        output = stdout + stderr
        expected_lines = {
            "IRONBANK:node:IRONBANK-PACKAGE-BYTES",
            "IRONBANK:pip:42",
            "IRONBANK:uv:uv:ironbank",
            "IRONBANK:npm:npm:realm",
            "IRONBANK:npx:npm:realm",
            "IRONBANK:apt:apt:ironbank-package-bytes",
            "IRONBANK:complete:apt+npm+npx+node+pip+uv",
        }
        assert expected_lines <= set(stdout.splitlines()), stdout
        if "IRONBANK:zstd:roundtrip" in stdout:
            assert "zstd:roundtrip" in stdout
        assert "No space left on device" not in output
        assert "Permission denied" not in output
        assert "externally-managed" not in output.lower()

        conn = _connect_session_db(service, session_id)
        try:
            exec_row = _eventually(
                lambda: conn.execute(
                    "SELECT * FROM exec_events WHERE command = ? ORDER BY id DESC LIMIT 1",
                    (f"bash /root/{script_name}",),
                ).fetchone(),
                lambda row: row is not None and row["exit_code"] == 0,
                timeout_s=20,
            )
            _assert_ledger_id(exec_row["event_id"])
            assert exec_row["source"] == "api"
            assert exec_row["stdout_bytes"] >= sum(len(line) for line in expected_lines)
            assert "IRONBANK:complete" in exec_row["stdout_preview"]
            assert exec_row["stderr_bytes"] >= 0
            assert exec_row["credential_ref"] is None

            package_audit_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM audit_events
                    WHERE argv LIKE '%pip install%'
                       OR argv LIKE '%uv pip install%'
                       OR argv LIKE '%npm install%'
                       OR argv LIKE '%apt-get install%'
                       OR exe LIKE '%/node'
                       OR exe LIKE '%/python3'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 4,
                timeout_s=20,
            )
            audit_text = "\n".join(f"{row['exe']} {row['argv']}" for row in package_audit_rows)
            assert "pip install --no-index" in audit_text
            assert "uv pip install" in audit_text
            assert "npm install -g" in audit_text
            assert "apt-get install" in audit_text
            for row in package_audit_rows[:20]:
                _assert_ledger_id(row["event_id"])
                assert row["pid"] > 0
                assert row["uid"] == 0
                assert row["credential_ref"] is None

            fs_rows = _eventually(
                lambda: conn.execute(
                    "SELECT * FROM fs_events WHERE path = ? OR path LIKE ? ORDER BY id",
                    (script_name, "ironbank-package-probe/%"),
                ).fetchall(),
                lambda rows: len(rows) >= 6,
                timeout_s=20,
            )
            paths = {row["path"] for row in fs_rows}
            assert script_name in paths
            assert "ironbank-package-probe/payload.txt" in paths
            assert any(path.endswith("package.json") for path in paths)
            assert any(path.endswith(".whl") for path in paths)
            for row in fs_rows[:80]:
                _assert_ledger_id(row["event_id"])
                assert row["path"]
                assert row["name"]
                assert row["directory"]
                assert row["action"] in {
                    "created",
                    "modified",
                    "deleted",
                    "import",
                    "export",
                    "read",
                    "restored",
                }

            security_rows = conn.execute(
                """
                SELECT *
                FROM security_rule_events
                WHERE event_id IN (
                    SELECT event_id FROM fs_events WHERE path = ?
                )
                  AND event_type = 'file.import'
                ORDER BY id
                """,
                (script_name,),
            ).fetchall()
            assert security_rows, "package probe upload must be governed by file rule"
            for row in security_rows:
                assert row["event_type"] == "file.import"
                assert row["rule_id"] == "profiles.rules.default_file"
                assert row["rule_action"] == "allow"
                event_json = json.loads(row["event_json"])
                assert event_json["file"]["import_name"] == script_name
                assert event_json["file"]["import_path"] == script_name
                assert event_json["decision"]["effective"] == "allow"
        finally:
            conn.close()

        history = client.get(f"/vms/{session_id}/history?layer=exec&limit=20", timeout=30)
        assert any(
            row["command"] == f"bash /root/{script_name}"
            and row["exit_code"] == 0
            and "IRONBANK:complete" in (row["stdout_preview"] or "")
            for row in history["commands"]
        )

        counts = client.get(f"/vms/{session_id}/history/counts", timeout=30)
        assert counts["exec_count"] >= 1
        assert counts["audit_count"] >= 4

        timeline = client.get(f"/vms/{session_id}/timeline?layers=exec,fs&limit=250", timeout=30)
        timeline_rows = _columnar_rows(timeline)
        assert {"exec", "fs"} <= {row["layer"] for row in timeline_rows}
        summaries = "\n".join(row["summary"] for row in timeline_rows)
        assert script_name in summaries
        assert "ironbank-package-probe" in summaries
    finally:
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        service.stop()
