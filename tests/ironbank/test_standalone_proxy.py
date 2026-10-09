"""Black-box acceptance for the VM-free OpenAI-compatible proxy."""

from __future__ import annotations

import json
import os
import selectors
import signal
import socket
import sqlite3
import subprocess
import textwrap
import time
from contextlib import closing
from pathlib import Path

from helpers.constants import BIN_DIR
from helpers.mock_server import start_mock_server, stop_process
from helpers.service import PROJECT_ROOT, ServiceInstance
from helpers.session_ledger import open_session_ledger

PYTHON_SECRET = "sk-capsem-standalone-python"
TYPESCRIPT_SECRET = "sk-capsem-standalone-typescript"


def _read_cli_startup(proc: subprocess.Popen[bytes]) -> tuple[str, str]:
    assert proc.stdout is not None
    selector = selectors.DefaultSelector()
    selector.register(proc.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + 15
    received = bytearray()
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise AssertionError(
                f"capsem proxy exited before startup: {proc.returncode}; output={received!r}"
            )
        for key, _ in selector.select(timeout=0.2):
            received.extend(os.read(key.fd, 4096))
        lines = received.decode(errors="replace").splitlines()
        session = next(
            (
                line.removeprefix("Proxy session: ")
                for line in lines
                if line.startswith("Proxy session: ")
            ),
            None,
        )
        base_url = next(
            (
                line.removeprefix("Base URL: ")
                for line in lines
                if line.startswith("Base URL: ")
            ),
            None,
        )
        if session and base_url:
            return session, base_url
    raise AssertionError(f"capsem proxy startup output timed out: {received!r}")


def _run_json(command: list[str]) -> dict[str, object]:
    completed = subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    lines = [line for line in completed.stdout.splitlines() if line.strip()]
    assert lines, completed.stdout + completed.stderr
    return json.loads(lines[-1])


def _records(path: Path) -> list[dict[str, object]]:
    if not path.exists():
        return []
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line
    ]


def _wait_for_model_rows(db_path: Path, minimum: int = 8) -> list[sqlite3.Row]:
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if db_path.exists():
            with closing(open_session_ledger(db_path)) as conn:
                conn.row_factory = sqlite3.Row
                rows = conn.execute("SELECT * FROM model_calls ORDER BY id").fetchall()
                if len(rows) >= minimum:
                    return rows
        time.sleep(0.25)
    raise AssertionError(
        f"standalone ledger did not reach {minimum} model rows: {db_path}"
    )


def _raw_http(port: int, request: bytes) -> bytes:
    with socket.create_connection(("127.0.0.1", port), timeout=3) as stream:
        stream.sendall(request)
        stream.shutdown(socket.SHUT_WR)
        chunks: list[bytes] = []
        while chunk := stream.recv(65536):
            chunks.append(chunk)
    return b"".join(chunks)


def _assert_no_raw_secret(db_path: Path, session_dir: Path) -> None:
    with closing(open_session_ledger(db_path)) as conn:
        tables = [
            row[0]
            for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name"
            )
        ]
        for table in tables:
            columns = conn.execute(f"PRAGMA table_info({table})").fetchall()
            text_columns = [
                row[1] for row in columns if str(row[2]).upper() in {"TEXT", ""}
            ]
            if not text_columns:
                continue
            selected = ", ".join(f'"{column}"' for column in text_columns)
            for row in conn.execute(f'SELECT {selected} FROM "{table}"').fetchall():
                rendered = "\n".join(str(value) for value in row)
                assert PYTHON_SECRET not in rendered
                assert TYPESCRIPT_SECRET not in rendered
    for path in session_dir.iterdir():
        if path.is_file() and path != db_path:
            content = path.read_bytes()
            assert PYTHON_SECRET.encode() not in content, path
            assert TYPESCRIPT_SECRET.encode() not in content, path


def _assert_sdk_result(result: dict[str, object], client: str) -> None:
    assert result == {
        "cancelled": True,
        "chat_stream": "Capsem ironbank poem",
        "chat_tool": "fixture_lookup",
        "chat_total_tokens": 456,
        "client": client,
        "responses_stream": "Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw",
        "responses_tool": "exec_command",
        "responses_total_tokens": 12,
        "upstream_code": "fixture_unavailable",
        "upstream_status": 503,
    }


def test_standalone_proxy_serves_official_sdks_without_a_vm():
    service = ServiceInstance(sign_binaries=False)
    mock_proc = None
    proxy_proc: subprocess.Popen[bytes] | None = None
    previous_corp = os.environ.get("CAPSEM_CORP_CONFIG")
    try:
        transcript = service.tmp_dir / "standalone-upstream.jsonl"
        mock_proc, ready = start_mock_server(request_log=transcript)
        corp_path = service.tmp_dir / "corp.toml"
        corp_path.write_text(
            textwrap.dedent(
                f"""
                refresh_policy = "24h"

                [network.upstream_overrides."api.openai.com:443"]
                dial = {json.dumps(ready["http_addr"])}
                protocol = "http"

                [corp.rules.allow_standalone_openai_fixture]
                name = "allow_standalone_openai_fixture"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Allow the hermetic standalone OpenAI acceptance fixture."
                match = 'http.host == "api.openai.com" && tcp.port == "443"'
                """
            ).strip()
            + "\n",
            encoding="utf-8",
        )
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp_path)
        service.start()
        client = service.client()
        assert client.get("/vms/list")["sandboxes"] == []

        proxy_env = os.environ.copy()
        proxy_env.update(
            {
                "CAPSEM_HOME": str(service.home_dir),
                "CAPSEM_RUN_DIR": str(service.tmp_dir),
                "HOME": str(service.home_dir),
            }
        )
        proxy_proc = subprocess.Popen(
            [str(BIN_DIR / "capsem"), "--uds-path", str(service.uds_path), "proxy"],
            cwd=PROJECT_ROOT,
            env=proxy_env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            bufsize=0,
        )
        session_id, base_url = _read_cli_startup(proxy_proc)
        assert session_id.startswith("proxy-")
        assert base_url.endswith("/v1")
        assert client.get("/vms/list")["sandboxes"] == []

        python_result = _run_json(
            [
                "uv",
                "run",
                "--project",
                "sdk/python",
                "--frozen",
                "python",
                "sdk/python/tests/standalone_openai_client.py",
                "--base-url",
                base_url,
            ]
        )
        typescript_result = _run_json(
            ["node", "sdk/typescript/tests/standalone-openai-client.ts", base_url]
        )
        _assert_sdk_result(python_result, "python")
        _assert_sdk_result(typescript_result, "typescript")

        port = int(base_url.split(":", 2)[2].split("/", 1)[0])
        before_connect = len(_records(transcript))
        connect_response = _raw_http(
            port,
            b"CONNECT attacker.invalid:443 HTTP/1.1\r\nHost: attacker.invalid:443\r\n\r\n",
        )
        assert not connect_response.startswith(b"HTTP/1.1 2")
        assert len(_records(transcript)) == before_connect, (
            "standalone endpoint must reject CONNECT instead of forwarding it"
        )

        absolute_response = _raw_http(
            port,
            b"GET http://attacker.invalid/secret HTTP/1.1\r\n"
            b"Host: attacker.invalid\r\nConnection: close\r\n\r\n",
        )
        assert not absolute_response.startswith(b"HTTP/1.1 2")
        assert len(_records(transcript)) == before_connect, (
            "standalone endpoint must reject absolute-form forward-proxy requests"
        )

        session_dir = service.tmp_dir / "sessions" / session_id
        db_path = session_dir / "session.db"
        model_rows = _wait_for_model_rows(db_path)
        assert {row["path"] for row in model_rows} == {
            "/v1/chat/completions",
            "/v1/responses",
        }
        assert {row["provider"] for row in model_rows} == {"openai"}
        assert {row["status_code"] for row in model_rows} >= {200, 503}
        assert any(
            row["input_tokens"] == 66 and row["output_tokens"] == 390
            for row in model_rows
        )
        assert any(
            row["input_tokens"] == 7 and row["output_tokens"] == 5 for row in model_rows
        )
        upstream = _records(transcript)
        assert {row["path"] for row in upstream} >= {
            "/v1/chat/completions",
            "/v1/responses",
        }
        headers = [row["headers"] for row in upstream]
        assert all(
            isinstance(value, dict) and value.get("host") == "api.openai.com"
            for value in headers
        )
        _assert_no_raw_secret(db_path, session_dir)

        proxy_proc.send_signal(signal.SIGTERM)
        assert proxy_proc.wait(timeout=15) == 0
        assert proxy_proc.stdout is not None
        proxy_proc.stdout.close()
        proxy_proc = None
        deadline = time.monotonic() + 5
        while session_dir.exists() and time.monotonic() < deadline:
            time.sleep(0.1)
        assert not session_dir.exists()
        with socket.socket() as probe:
            probe.settimeout(1)
            assert probe.connect_ex(("127.0.0.1", port)) != 0
        assert client.get("/vms/list")["sandboxes"] == []
    finally:
        if proxy_proc is not None:
            proxy_proc.kill()
            proxy_proc.wait(timeout=5)
            if proxy_proc.stdout is not None:
                proxy_proc.stdout.close()
        service.stop()
        stop_process(mock_proc)
        if previous_corp is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = previous_corp
