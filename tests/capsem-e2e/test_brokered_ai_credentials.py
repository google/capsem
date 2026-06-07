"""Brokered AI credential VM invariants."""

import json
import os
import shlex
import sqlite3
import time
import uuid
from pathlib import Path

import blake3
import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.e2e


def _credential_ref(provider: str, raw: str) -> str:
    hasher = blake3.blake3()
    hasher.update(b"capsem.credential.v1")
    hasher.update(b"\0")
    hasher.update(provider.encode())
    hasher.update(b"\0")
    hasher.update(raw.encode())
    return f"credential:blake3:{hasher.hexdigest()}"


def _write_brokered_settings(tmp_dir: Path) -> dict[str, str]:
    raw_anthropic = "sk-ant-e2e-raw-secret"
    raw_google = "AIza-e2e-raw-secret"
    refs = {
        "anthropic": _credential_ref("anthropic", raw_anthropic),
        "google": _credential_ref("google", raw_google),
    }
    (tmp_dir / "credential-store.json").write_text(
        json.dumps(
            {
                f"anthropic:{refs['anthropic']}": raw_anthropic,
                f"google:{refs['google']}": raw_google,
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    (tmp_dir / "user.toml").write_text(
        f"""
[settings]
"ai.anthropic.allow" = {{ value = true, modified = "2026-06-05T00:00:00Z" }}
"ai.anthropic.api_key" = {{ value = "{refs['anthropic']}", modified = "2026-06-05T00:00:00Z" }}
"ai.google.allow" = {{ value = true, modified = "2026-06-05T00:00:00Z" }}
"ai.google.api_key" = {{ value = "{refs['google']}", modified = "2026-06-05T00:00:00Z" }}
""".lstrip(),
        encoding="utf-8",
    )
    return refs


def _vm_name(prefix: str) -> str:
    return f"{prefix}-{uuid.uuid4().hex[:8]}"


def _delete_vm(svc: ServiceInstance, vm: str) -> None:
    try:
        svc.client().delete(f"/vms/{vm}/delete", timeout=60)
    except Exception:
        pass


def _session_db(svc: ServiceInstance, vm: str) -> Path:
    return svc.tmp_dir / "sessions" / vm / "session.db"


def _guest_python(script: str) -> str:
    return f"python3 -c {shlex.quote(script)}"


def _wait_for_net_credential_ref(db_path: Path, credential_ref: str, timeout: float = 20.0):
    deadline = time.time() + timeout
    last_rows = []
    while time.time() < deadline:
        if db_path.exists():
            conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
            conn.row_factory = sqlite3.Row
            try:
                last_rows = conn.execute(
                    "SELECT domain, credential_ref, request_headers FROM net_events"
                ).fetchall()
                for row in last_rows:
                    if row["credential_ref"] == credential_ref:
                        return row
            finally:
                conn.close()
        time.sleep(0.2)
    pytest.fail(f"timed out waiting for credential_ref; rows={[dict(r) for r in last_rows]}")


def test_brokered_claude_and_gemini_refs_are_guest_visible_without_raw_secrets(monkeypatch):
    svc = ServiceInstance()
    vm = None
    refs = _write_brokered_settings(svc.tmp_dir)
    monkeypatch.setenv("CAPSEM_USER_CONFIG", str(svc.tmp_dir / "user.toml"))
    monkeypatch.setenv(
        "CAPSEM_CREDENTIAL_BROKER_TEST_STORE",
        str(svc.tmp_dir / "credential-store.json"),
    )

    try:
        svc.start()
        vm = _vm_name("brokered-ai")
        svc.client().post(
            "/vms/create",
            {
                "name": vm,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "persistent": False,
            },
            timeout=120,
        )
        assert wait_exec_ready(svc.client(), vm)

        inspect_script = r"""
import json
import os
from pathlib import Path

paths = [Path("/root/.claude.json"), Path("/root/.gemini/settings.json")]
payload = {
    "anthropic_env": os.environ.get("ANTHROPIC_API_KEY"),
    "gemini_env": os.environ.get("GEMINI_API_KEY"),
    "google_env": os.environ.get("GOOGLE_API_KEY"),
    "files": {str(p): p.read_text(errors="replace") if p.exists() else "" for p in paths},
}
print(json.dumps(payload))
"""
        result = svc.client().post(
            f"/exec/{vm}",
            {"command": _guest_python(inspect_script), "timeout_secs": 30},
            timeout=40,
        )
        assert result["exit_code"] == 0, result
        payload = json.loads(result["stdout"])
        assert payload["anthropic_env"] == refs["anthropic"]
        assert payload["gemini_env"] == refs["google"]
        assert payload["google_env"] in (None, "")
        serialized = json.dumps(payload)
        assert "sk-ant-e2e-raw-secret" not in serialized
        assert "AIza-e2e-raw-secret" not in serialized

        for cli in ("claude", "gemini"):
            cli_result = svc.client().post(
                f"/exec/{vm}",
                {"command": f"{cli} --help >/tmp/{cli}.help 2>&1; echo rc=$?", "timeout_secs": 20},
                timeout=30,
            )
            assert cli_result["exit_code"] == 0, cli_result
            assert "rc=0" in cli_result["stdout"], cli_result

        db_path = _session_db(svc, vm)
        curl_result = svc.client().post(
            f"/exec/{vm}",
            {
                "command": (
                    "curl -sS --max-time 15 -o /dev/null "
                    "-H \"x-api-key: $ANTHROPIC_API_KEY\" "
                    "-H \"anthropic-version: 2023-06-01\" "
                    "-H \"content-type: application/json\" "
                    "https://api.anthropic.com/v1/messages "
                    "-d '{\"model\":\"claude-3-haiku-20240307\",\"max_tokens\":1,\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}' "
                    "2>/tmp/anthropic.err || true"
                ),
                "timeout_secs": 30,
            },
            timeout=45,
        )
        assert curl_result["exit_code"] == 0, curl_result
        row = _wait_for_net_credential_ref(db_path, refs["anthropic"])
        assert row["domain"] == "api.anthropic.com"
        assert refs["anthropic"] in row["request_headers"]
        assert "sk-ant-e2e-raw-secret" not in row["request_headers"]
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()
