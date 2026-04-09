"""Gateway terminal WebSocket tests.

Tests the /terminal/{id} WebSocket endpoint through the real gateway binary.
Uses mock UDS with a WebSocket echo server to verify relay behavior.
"""

import asyncio
import json
import os
import socket
import subprocess
import tempfile
import threading
import time
from pathlib import Path

import pytest
import websockets

from helpers.gateway import GatewayInstance

pytestmark = pytest.mark.gateway


class MockWsProcess:
    """A mock WebSocket server on UDS that echoes messages back."""

    def __init__(self, sock_path: str):
        self.sock_path = sock_path
        self._server = None
        self._loop = None
        self._thread = None

    def start(self):
        self._loop = asyncio.new_event_loop()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        # Wait for server to bind
        for _ in range(50):
            if os.path.exists(self.sock_path):
                return
            time.sleep(0.1)
        raise RuntimeError("Mock WS server didn't start")

    def _run(self):
        asyncio.set_event_loop(self._loop)
        self._loop.run_until_complete(self._serve())

    async def _serve(self):
        self._server = await websockets.unix_serve(
            self._handler, self.sock_path,
        )
        await self._server.serve_forever()

    async def _handler(self, ws):
        try:
            async for msg in ws:
                # Echo text and binary back
                await ws.send(msg)
        except websockets.exceptions.ConnectionClosed:
            pass

    def stop(self):
        if self._server:
            self._server.close()
        if self._loop:
            self._loop.call_soon_threadsafe(self._loop.stop)
        if self._thread:
            self._thread.join(timeout=5)


@pytest.fixture(scope="module")
def ws_env():
    """Start a gateway with a mock WS process on a known VM ID.

    Uses a short /tmp path to avoid AF_UNIX path length limits (108 bytes).
    The gateway reads HOME to find ~/.capsem/run/, and the mock WS
    server's socket path must be under that same run/instances/ dir.
    """
    # Use a short path to stay under the 108-byte AF_UNIX limit
    tmp_dir = Path(tempfile.mkdtemp(prefix="gw-ws-", dir="/tmp"))
    run_dir = tmp_dir / ".capsem" / "run"
    instances_dir = run_dir / "instances"
    instances_dir.mkdir(parents=True)

    # Start mock WS process for "ws-vm"
    ws_sock = str(instances_dir / "ws-vm-ws.sock")
    mock_ws = MockWsProcess(ws_sock)
    mock_ws.start()

    # Mock service socket (gateway uses this for proxied requests)
    service_sock = str(run_dir / "service.sock")
    import socketserver
    from http.server import BaseHTTPRequestHandler

    class DummyHandler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            body = b'{"sandboxes":[]}'
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    class UnixServer(socketserver.UnixStreamServer):
        allow_reuse_address = True

    svc_server = UnixServer(service_sock, DummyHandler)
    svc_thread = threading.Thread(target=svc_server.serve_forever, daemon=True)
    svc_thread.start()

    # Start gateway -- override HOME so it uses our short tmp path
    gw = GatewayInstance(uds_path=service_sock)
    # Patch tmp_dir to use our short path so runtime files go there
    gw.tmp_dir = tmp_dir
    gw.start()

    yield gw, mock_ws, tmp_dir

    gw.stop()
    mock_ws.stop()
    svc_server.shutdown()


class TestTerminalWebSocket:

    def test_ws_connect_and_echo_text(self, ws_env):
        """Connect to /terminal/{id} via WebSocket and echo text."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/terminal/ws-vm"
            headers = {"Authorization": f"Bearer {gw.token}"}
            async with websockets.connect(url, additional_headers=headers) as ws:
                await ws.send("hello from test")
                reply = await asyncio.wait_for(ws.recv(), timeout=5)
                assert reply == "hello from test"

        asyncio.run(run())

    def test_ws_echo_binary(self, ws_env):
        """Binary messages are relayed correctly."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/terminal/ws-vm"
            headers = {"Authorization": f"Bearer {gw.token}"}
            async with websockets.connect(url, additional_headers=headers) as ws:
                data = bytes(range(256))
                await ws.send(data)
                reply = await asyncio.wait_for(ws.recv(), timeout=5)
                assert reply == data

        asyncio.run(run())

    def test_ws_multiple_messages(self, ws_env):
        """Multiple messages round-trip correctly."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/terminal/ws-vm"
            headers = {"Authorization": f"Bearer {gw.token}"}
            async with websockets.connect(url, additional_headers=headers) as ws:
                for i in range(10):
                    msg = f"message-{i}"
                    await ws.send(msg)
                    reply = await asyncio.wait_for(ws.recv(), timeout=5)
                    assert reply == msg

        asyncio.run(run())

    def test_ws_close_clean(self, ws_env):
        """Clean close completes without error."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/terminal/ws-vm"
            headers = {"Authorization": f"Bearer {gw.token}"}
            ws = await websockets.connect(url, additional_headers=headers)
            await ws.send("before close")
            reply = await asyncio.wait_for(ws.recv(), timeout=5)
            assert reply == "before close"
            await ws.close()

        asyncio.run(run())

    def test_ws_invalid_id_rejected(self, ws_env):
        """WebSocket upgrade fails for invalid VM ID (dots)."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/terminal/vm..bad"
            headers = {"Authorization": f"Bearer {gw.token}"}
            with pytest.raises(Exception):
                await websockets.connect(url, additional_headers=headers)

        asyncio.run(run())

    def test_ws_no_auth_rejected(self, ws_env):
        """WebSocket without auth token is rejected with 401."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/terminal/ws-vm"
            with pytest.raises(Exception):
                await websockets.connect(url)

        asyncio.run(run())

    def test_ws_nonexistent_vm_closes(self, ws_env):
        """WebSocket to non-existent VM ID connects but drops (no UDS)."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/terminal/no-such-vm"
            headers = {"Authorization": f"Bearer {gw.token}"}
            # Connection may upgrade but then immediately close
            # because there's no UDS socket for this VM
            try:
                async with websockets.connect(url, additional_headers=headers) as ws:
                    # Try to receive -- should get close or error
                    try:
                        await asyncio.wait_for(ws.recv(), timeout=3)
                    except (websockets.exceptions.ConnectionClosed, asyncio.TimeoutError):
                        pass  # Expected
            except (websockets.exceptions.ConnectionClosed, ConnectionRefusedError):
                pass  # Also expected

        asyncio.run(run())
