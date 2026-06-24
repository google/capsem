"""Ironbank black-box agent bootstrap tests.

These tests prove the profile-projected agent bootstrap surface from outside
the product: service routes, guest-visible files, command output, and the
session ledger. They intentionally do not inspect Rust internals.
"""

from __future__ import annotations

import json
import re
import sqlite3
import textwrap
import time
import uuid

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready, vm_name

pytestmark = pytest.mark.integration

SECRET_MARKER_RE = re.compile(
    r"(sk-[A-Za-z0-9_-]{20,}|ghp_[A-Za-z0-9_]{20,}|AIza[0-9A-Za-z_-]{20,}|"
    r"refresh_token|access_token|id_token|authorization_code)",
    re.IGNORECASE,
)

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
    "credential_ref",
}


def _connect_session_db(service: ServiceInstance, session_id: str) -> sqlite3.Connection:
    db_path = service.tmp_dir / "sessions" / session_id / "session.db"
    assert db_path.exists(), f"session.db missing at {db_path}"
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


def _agent_bootstrap_probe_script() -> str:
    return textwrap.dedent(
        r'''
        import json
        import os
        import re
        import shutil
        import stat
        import subprocess
        from pathlib import Path

        secret_re = re.compile(
            r"(sk-[A-Za-z0-9_-]{20,}|ghp_[A-Za-z0-9_]{20,}|AIza[0-9A-Za-z_-]{20,}|"
            r"refresh_token|access_token|id_token|authorization_code)",
            re.IGNORECASE,
        )
        forbidden_path_re = re.compile(r"(token|oauth|conversation|history|cache|log)", re.IGNORECASE)
        config_paths = {
            "agy_config": Path("/root/.antigravity/config.json"),
            "agy_settings": Path("/root/.antigravity/settings.json"),
            "agy_cli_settings": Path("/root/.gemini/antigravity-cli/settings.json"),
            "agy_product_config": Path("/root/.gemini/config/config.json"),
            "claude_json": Path("/root/.claude.json"),
            "claude_settings": Path("/root/.claude/settings.json"),
            "claude_settings_local": Path("/root/.claude/settings.local.json"),
            "codex_config": Path("/root/.codex/config.toml"),
            "gemini_installation_id": Path("/root/.gemini/installation_id"),
            "gemini_projects": Path("/root/.gemini/projects.json"),
            "gemini_settings": Path("/root/.gemini/settings.json"),
            "gemini_trusted_folders": Path("/root/.gemini/trustedFolders.json"),
            "root_mcp": Path("/root/.mcp.json"),
        }

        def read_text(path):
            return path.read_text(encoding="utf-8")

        missing = [name for name, path in config_paths.items() if not path.exists()]
        assert not missing, missing

        raw_config = {name: read_text(path) for name, path in config_paths.items()}
        for name, text in raw_config.items():
            assert not secret_re.search(text), name

        agy_settings = json.loads(raw_config["agy_settings"])
        assert agy_settings["colorScheme"] == "dark"
        assert "/root" in agy_settings["trustedWorkspaces"]

        agy_config = json.loads(raw_config["agy_config"])
        assert "ai" not in agy_config, agy_config
        agy_product_config = json.loads(raw_config["agy_product_config"])
        assert "ai" not in agy_product_config, agy_product_config
        agy_cli_settings = json.loads(raw_config["agy_cli_settings"])
        assert "toolPermission" not in agy_cli_settings
        assert "/root" in agy_cli_settings["trustedWorkspaces"]
        assert agy_cli_settings["telemetry"]["enabled"] is False
        assert agy_cli_settings["autoUpdate"]["enabled"] is False

        claude_json = json.loads(raw_config["claude_json"])
        assert claude_json["hasCompletedOnboarding"] is True
        assert claude_json["hasTrustDialogAccepted"] is True
        assert claude_json["projects"]["/root"]["hasTrustDialogAccepted"] is True

        claude_settings = json.loads(raw_config["claude_settings"])
        assert claude_settings["permissions"]["defaultMode"] == "bypassPermissions"
        assert claude_settings["env"]["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"] == "1"

        claude_local = json.loads(raw_config["claude_settings_local"])
        assert claude_local["enabledMcpjsonServers"] == ["capsem"]

        assert 'command = "/run/capsem-mcp-server"' in raw_config["codex_config"]

        gemini_settings = json.loads(raw_config["gemini_settings"])
        assert gemini_settings["general"]["enableAutoUpdate"] is False
        assert gemini_settings["privacy"]["usageStatisticsEnabled"] is False
        assert gemini_settings["privacy"]["sessionRetention"] == "none"
        assert gemini_settings["telemetry"]["enabled"] is False
        assert gemini_settings["security"]["auth"]["selectedType"] == "gemini-api-key"
        assert gemini_settings["security"]["folderTrust.enabled"] is False

        gemini_projects = json.loads(raw_config["gemini_projects"])
        assert gemini_projects["projects"]["/root"] == "root"
        gemini_trusted = json.loads(raw_config["gemini_trusted_folders"])
        assert gemini_trusted["/root"] == "TRUST_FOLDER"
        assert raw_config["gemini_installation_id"].strip()

        root_mcp = json.loads(raw_config["root_mcp"])
        assert root_mcp["mcpServers"]["capsem"]["command"] == "/run/capsem-mcp-server"

        scan_roots = [
            Path("/root/.antigravity"),
            Path("/root/.claude"),
            Path("/root/.codex"),
            Path("/root/.gemini"),
        ]
        forbidden_before = []
        for root in scan_roots:
            if not root.exists():
                continue
            for path in root.rglob("*"):
                rel = str(path.relative_to("/root"))
                if forbidden_path_re.search(rel):
                    forbidden_before.append(rel)
        assert forbidden_before == [], forbidden_before

        commands = {}
        for name in ["agy", "claude", "codex", "gemini"]:
            path = shutil.which(name)
            assert path, f"{name} missing from PATH"
            st = os.stat(path)
            assert st.st_mode & stat.S_IXUSR, f"{name} is not executable"
            commands[name] = {
                "path": path,
                "realpath": os.path.realpath(path),
            }

        assert commands["agy"]["path"] == "/usr/local/bin/agy"
        agy_wrapper = Path(commands["agy"]["path"]).read_text(encoding="utf-8")
        assert "agy-real --dangerously-skip-permissions" in agy_wrapper
        assert Path("/usr/local/bin/agy-real").exists()
        assert os.access("/usr/local/bin/agy-real", os.X_OK)
        assert commands["gemini"]["path"].endswith("/gemini")
        gemini_wrapper = Path(commands["gemini"]["path"]).read_text(encoding="utf-8")
        assert "cleanup_gemini_runtime_state" in gemini_wrapper
        gemini_real = Path(commands["gemini"]["path"]).parent / "gemini-real"
        assert gemini_real.exists()
        assert gemini_real.is_symlink()
        assert os.access(gemini_real, os.X_OK)

        help_outputs = {}
        for name in ["claude", "codex", "gemini"]:
            result = subprocess.run(
                [name, "--help"],
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                timeout=30,
            )
            output = result.stdout
            assert result.returncode == 0, {"name": name, "returncode": result.returncode, "output": output[:600]}
            for marker in ["SyntaxError", "TypeError", "ReferenceError", "Cannot find module"]:
                assert marker not in output, {"name": name, "marker": marker, "output": output[:600]}
            help_outputs[name] = output[:240]

        agy_version = subprocess.run(
            ["agy", "--version"],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=30,
        )
        assert agy_version.returncode == 0, agy_version.stdout[:600]
        assert "dangerously" not in agy_version.stdout.lower()
        help_outputs["agy"] = agy_version.stdout[:240]

        forbidden_after = []
        for root in scan_roots:
            if not root.exists():
                continue
            for path in root.rglob("*"):
                rel = str(path.relative_to("/root"))
                if forbidden_path_re.search(rel):
                    forbidden_after.append(rel)
        assert forbidden_after == [], forbidden_after

        result = {
            "commands": commands,
            "help_outputs": help_outputs,
            "config_paths": {name: str(path) for name, path in config_paths.items()},
            "forbidden_before": forbidden_before,
            "forbidden_after": forbidden_after,
        }
        print("IRONBANK_AGENT_BOOTSTRAP_RESULT=" + json.dumps(result, sort_keys=True))
        '''
    ).strip()


def test_profile_agent_bootstrap_pays_ledger_debt_blackbox():
    service = ServiceInstance()
    session_id = vm_name("ironbank-agent")
    script_name = f"ironbank-agent-bootstrap-{uuid.uuid4().hex[:8]}.py"
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

        script_bytes = _agent_bootstrap_probe_script().encode()
        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path={script_name}",
            script_bytes,
            timeout=30,
        )
        assert upload is not None, "script upload returned no body"
        assert upload.get("success") is True, f"script upload failed: {upload}"
        assert upload.get("size") == len(script_bytes)

        status_before = client.get(f"/vms/{session_id}/status", timeout=30)
        assert status_before is not None
        assert status_before.get("id") == session_id or status_before.get("name") == session_id
        assert status_before.get("status") == "Running"
        assert status_before.get("available_actions") == ["pause", "stop", "fork", "delete"]

        info_before = client.get(f"/vms/{session_id}/info", timeout=30)
        assert info_before is not None
        assert info_before.get("id") == session_id or info_before.get("name") == session_id
        assert info_before.get("profile_id") == CODE_PROFILE_ID
        assert info_before.get("status") == "Running"

        exec_resp = client.post(
            f"/vms/{session_id}/exec",
            {"command": f"python3 /root/{script_name}", "timeout_secs": 180},
            timeout=210,
        )
        assert exec_resp is not None, "exec returned no body"
        assert exec_resp.get("exit_code") == 0, exec_resp
        combined = exec_resp.get("stdout", "") + exec_resp.get("stderr", "")
        assert "IRONBANK_AGENT_BOOTSTRAP_RESULT=" in combined
        assert not SECRET_MARKER_RE.search(combined), combined
        result_line = next(
            line for line in exec_resp.get("stdout", "").splitlines()
            if line.startswith("IRONBANK_AGENT_BOOTSTRAP_RESULT=")
        )
        probe = json.loads(result_line.split("=", 1)[1])
        assert set(probe["commands"]) == {"agy", "claude", "codex", "gemini"}
        assert probe["commands"]["agy"]["path"] == "/usr/local/bin/agy"
        assert probe["forbidden_before"] == []
        assert probe["forbidden_after"] == []

        history = client.get(f"/vms/{session_id}/history", timeout=30)
        assert history is not None
        assert history.get("total", 0) >= 1
        command_text = " ".join(
            (entry.get("command") or "") + " " + (entry.get("stdout_preview") or "")
            for entry in history.get("commands", [])
        )
        assert script_name in command_text
        assert "IRONBANK_AGENT_BOOTSTRAP_RESULT" in command_text

        counts = client.get(f"/vms/{session_id}/history/counts", timeout=30)
        assert counts is not None
        assert isinstance(counts.get("exec_count"), int) and counts["exec_count"] >= 1
        assert isinstance(counts.get("audit_count"), int) and counts["audit_count"] >= 0

        conn = _connect_session_db(service, session_id)
        try:
            assert _table_columns(conn, "exec_events") == EXPECTED_EXEC_COLUMNS
            exec_row = _eventually(
                lambda: conn.execute(
                    "SELECT * FROM exec_events WHERE command = ? ORDER BY id DESC LIMIT 1",
                    (f"python3 /root/{script_name}",),
                ).fetchone(),
                lambda row: row is not None and row["exit_code"] == 0,
                timeout_s=15,
            )
            assert exec_row["source"] == "api"
            assert re.fullmatch(r"[0-9a-f]{12}", exec_row["event_id"])
            assert exec_row["stdout_bytes"] >= len("IRONBANK_AGENT_BOOTSTRAP_RESULT")
            assert exec_row["stderr_bytes"] >= 0
            assert "IRONBANK_AGENT_BOOTSTRAP_RESULT" in (exec_row["stdout_preview"] or "")
            assert exec_row["credential_ref"] is None
        finally:
            conn.close()
    finally:
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        service.stop()
