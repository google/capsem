"""Real local TCP source contracts; no installed/native gateway claim."""

import contextlib
import http.server
import json
import os
import socket
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from build_system.tests.release.test_installed_apply_probe import arguments, probe

TOKEN = "A0b1C2d3" * 8


@contextlib.contextmanager
def receiver(tmp_path, mode="success"):
    runtime = tmp_path / "run"
    runtime.mkdir(mode=0o700)
    calls = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, format, *args):
            pass

        def send(self, value, status=200):
            data = json.dumps(value).encode()
            self.send_response(status)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            calls.append(("GET", self.path, self.headers.get("Authorization")))
            if self.headers.get("Authorization") != f"Bearer {TOKEN}":
                self.send({"error": "private body"}, 403)
                return
            if mode == "rotate":
                (runtime / "gateway.pid").write_text(str(os.getpid() + 1))
            self.send(
                {"tree": [{"kind": "leaf", "id": "app.auto_update", "effective_value": False}]}
            )

        def do_POST(self):
            calls.append(("POST", self.path, self.headers.get("Authorization")))
            assert self.path == "/update/apply"
            assert json.loads(self.rfile.read(int(self.headers["Content-Length"]))) == {
                "confirmed": True
            }
            if mode in {"eof", "reset"}:
                if mode == "reset":
                    self.connection.setsockopt(
                        socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0)
                    )
                self.close_connection = True
                return
            if mode in {"truncated", "oversize", "stall", "chunk_missing"}:
                self.send_response(200)
                if mode == "chunk_missing":
                    self.send_header("Transfer-Encoding", "chunked")
                else:
                    self.send_header("Content-Length", "9000" if mode == "oversize" else "99")
                self.end_headers()
                if mode == "stall":
                    time.sleep(0.12)
                else:
                    self.wfile.write(b"2\r\n{}\r\n0\r\n" if mode == "chunk_missing" else b"{}")
                self.close_connection = True
                return
            self.send(
                {
                    "status": "failed" if mode == "failed" else "succeeded",
                    "exit_code": int(mode == "failed"),
                    "private": "response must not escape",
                },
                500 if mode == "http500" else 200,
            )

    class Server(http.server.ThreadingHTTPServer):
        daemon_threads = False

    with Server(("127.0.0.1", 0), Handler) as server:
        values = {
            "gateway.token": TOKEN,
            "gateway.port": str(server.server_port),
            "gateway.pid": str(os.getpid()),
            "service.pid": str(os.getpid()),
        }
        for name, value in values.items():
            (runtime / name).write_text(value)
            (runtime / name).chmod(0o600)
        thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.01))
        thread.start()
        try:
            yield runtime, calls
        finally:
            server.shutdown()
            thread.join(timeout=1)
            assert not thread.is_alive()


@pytest.mark.parametrize(
    "mode,outcome",
    [
        ("success", "succeeded"),
        ("failed", "application_failure"),
        ("http500", "http_failure"),
        *[
            (value, "transport_failure")
            for value in ("eof", "reset", "truncated", "oversize", "stall", "chunk_missing")
        ],
    ],
)
def test_original_authenticated_tcp_request_is_never_replayed(tmp_path, mode, outcome, capsys):
    from helpers.installed_gateway import GatewayTransport

    with receiver(tmp_path, mode) as (runtime, calls):
        owner = GatewayTransport(runtime, maximum=4096)
        try:
            args = arguments(tmp_path)
            args.transport = "gateway_http"
            args.timeout = 0.05 if mode == "stall" else 1
            result = probe().request_apply(owner, args)
            assert result["transport"] == "gateway_http"
            assert result["outcome"] == outcome
            assert result["attempts"] == 1
            assert [method for method, _, _ in calls] == ["GET", "POST"]
            assert all(auth == f"Bearer {TOKEN}" for _, _, auth in calls)
            assert TOKEN not in repr(owner)
            assert TOKEN not in args.output.read_text()
            assert "response must not escape" not in args.output.read_text()
        finally:
            owner.close()
    assert TOKEN not in str(capsys.readouterr())


@pytest.mark.parametrize(
    "damage",
    [
        "missing",
        "malformed_port",
        "malformed_pid",
        "empty_token",
        "oversize",
        "symlink",
        "writable",
        "foreign_token",
        "rotate",
    ],
)
def test_missing_foreign_or_rotated_markers_cannot_authorize_an_apply(tmp_path, damage):
    from helpers.installed_gateway import GatewayTransport

    with receiver(tmp_path, "rotate" if damage == "rotate" else "success") as (runtime, calls):
        if damage == "missing":
            (runtime / "gateway.pid").unlink()
        elif damage == "malformed_port":
            (runtime / "gateway.port").write_text("65536")
        elif damage == "malformed_pid":
            (runtime / "gateway.pid").write_text("9999999999999999999")
        elif damage == "empty_token":
            (runtime / "gateway.token").write_text("")
        elif damage == "oversize":
            (runtime / "gateway.token").write_text("s" * 5000)
        elif damage == "symlink":
            (runtime / "gateway.token").unlink()
            (runtime / "gateway.token").symlink_to(runtime / "service.pid")
        elif damage == "writable":
            (runtime / "gateway.token").chmod(0o666)
        elif damage == "foreign_token":
            (runtime / "gateway.token").write_text("X" * 64)
        owner = None
        try:
            with pytest.raises(ValueError):
                owner = GatewayTransport(runtime, maximum=4096)
                args = arguments(tmp_path)
                args.transport = "gateway_http"
                probe().request_apply(owner, args)
            assert not any(method == "POST" for method, _, _ in calls)
            assert not (tmp_path / "request.json").exists()
        finally:
            if owner is not None:
                owner.close()


def test_fifo_marker_cannot_block_before_owned_regular_file_admission(tmp_path):
    with receiver(tmp_path) as (runtime, calls):
        token = runtime / "gateway.token"
        token.unlink()
        os.mkfifo(token, 0o600)
        script = """import sys
from pathlib import Path
from helpers.installed_gateway import GatewayTransport
try:
    GatewayTransport(Path(sys.argv[1]), maximum=4096)
except ValueError:
    raise SystemExit(0)
raise SystemExit(2)
"""
        result = subprocess.run(
            [sys.executable, "-c", script, str(runtime)],
            cwd=Path(__file__).resolve().parents[3] / "tests",
            capture_output=True,
            text=True,
            timeout=1,
            check=False,
        )
        assert result.returncode == 0
        assert calls == []


def test_actual_cli_selects_gateway_once_and_rejects_ambiguous_address(tmp_path, monkeypatch):
    with receiver(tmp_path) as (runtime, calls):
        args = arguments(tmp_path)
        marker = tmp_path / "marker"
        marker.write_text("0")
        argv = [
            "probe",
            "request",
            "--gateway-run-dir",
            str(runtime),
            "--output",
            str(args.output),
            "--timeout",
            "1",
            "--maximum",
            "4096",
            "--source",
            args.source,
            "--digest",
            args.digest,
            "--source-commit",
            args.source_commit,
            "--audit",
            str(args.audit),
            "--after-line-file",
            str(marker),
        ]
        monkeypatch.setattr(sys, "argv", [*argv, "--socket", "never-opened"])
        with pytest.raises(SystemExit) as refused:
            probe().main()
        assert refused.value.code == 2
        assert calls == []
        monkeypatch.setattr(sys, "argv", argv)
        assert probe().main() == 0
        assert [method for method, _, _ in calls] == ["GET", "POST"]
        assert json.loads(args.output.read_text())["transport"] == "gateway_http"


def test_linux_original_owner_selects_gateway_without_second_request(tmp_path):
    import shlex
    from pathlib import Path

    from capsem_builder.release.tools.release_installed_apply import shell

    root = Path(__file__).resolve().parents[3]
    request = shlex.split(shell(root, "request", tmp_path / "receipt"))
    control = shlex.split(shell(root, "disable-automatic", tmp_path / "control"))
    assert request.count("request") == 1
    assert "--gateway-run-dir" in request and "--socket" not in request
    assert "--socket" in control and "--gateway-run-dir" not in control
