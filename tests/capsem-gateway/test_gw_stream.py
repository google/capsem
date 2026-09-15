"""Gateway stream tunnel tests.

Drives `/vms/{id}/stream` through the real gateway binary against a mock
service socket that speaks WebSocket, so the tunnel is proved end to end:
authentication, subprotocol negotiation, byte-exact relay, and refusal.
"""

import asyncio
import os
import tempfile
import threading
import time
from http import HTTPStatus
from pathlib import Path

import pytest
import websockets
from helpers.gateway import GatewayInstance
from websockets.typing import Subprotocol

pytestmark = pytest.mark.gateway


STREAM_SUBPROTOCOL = Subprotocol("capsem.stream.v1")


class MockWsProcess:
    """A mock service on UDS: `/vms/ws-vm/stream` echoes frames back.

    Plain HTTP requests (the gateway's status probe) get an empty VM list, and
    a stream for any other VM gets the service's 404.
    """

    def __init__(self, sock_path: str):
        self.sock_path = sock_path
        self._server = None
        self._loop = None
        self._thread = None
        self._shutdown = None
        self._ready = threading.Event()
        self._startup_error = None

    def start(self):
        self._loop = asyncio.new_event_loop()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        # Wait until the worker has both bound the socket and installed the
        # shutdown event. The socket can appear before _shutdown is assigned,
        # and stop() must never race that tiny window.
        if self._ready.wait(timeout=5) and os.path.exists(self.sock_path):
            return
        if self._startup_error is not None:
            raise RuntimeError(
                "Mock WS server failed to start"
            ) from self._startup_error
        raise RuntimeError("Mock WS server didn't start")

    def _run(self):
        asyncio.set_event_loop(self._loop)
        try:
            self._loop.run_until_complete(self._serve())
        except BaseException as exc:
            self._startup_error = exc
            self._ready.set()
            raise
        finally:
            self._loop.close()

    async def _serve(self):
        self._server = await websockets.unix_serve(
            self._handler,
            self.sock_path,
            subprotocols=[STREAM_SUBPROTOCOL],
            process_request=self._route,
        )
        # Park on an Event instead of serve_forever(): serve_forever() only
        # returns on task cancellation, which complicates cross-thread shutdown.
        self._shutdown = asyncio.Event()
        self._ready.set()
        try:
            await self._shutdown.wait()
        finally:
            self._server.close()
            await self._server.wait_closed()

    @staticmethod
    def _route(connection, request):
        if request.headers.get("Upgrade", "").lower() != "websocket":
            response = connection.respond(HTTPStatus.OK, '{"sandboxes":[]}')
            response.headers["Content-Type"] = "application/json"
            return response
        if request.path != "/vms/ws-vm/stream":
            return connection.respond(HTTPStatus.NOT_FOUND, "VM not found")
        return None

    async def _handler(self, ws):
        try:
            async for msg in ws:
                # Echo text and binary back
                await ws.send(msg)
        except websockets.exceptions.ConnectionClosed:
            pass

    def stop(self):
        # Signal shutdown on the loop thread so _serve() can close the
        # server cleanly and run_until_complete() can return on its own.
        # Calling loop.stop() here would kill the loop mid-await and raise
        # "Event loop stopped before Future completed." on the worker thread.
        if self._loop is not None and self._shutdown is not None:
            self._loop.call_soon_threadsafe(self._shutdown.set)
        if self._thread is not None:
            self._thread.join(timeout=5)


@pytest.fixture(scope="module")
def ws_env():
    """Start a gateway in front of a mock service that knows VM `ws-vm`.

    Uses a short /tmp path to avoid AF_UNIX path length limits (104 bytes).
    """
    # Use a short path to stay under the 108-byte AF_UNIX limit
    tmp_dir = Path(tempfile.mkdtemp(prefix="capsem-gw-ws-", dir="/tmp"))
    run_dir = tmp_dir / ".capsem" / "run"
    run_dir.mkdir(parents=True)

    # The mock service answers both proxied requests and stream upgrades.
    service_sock = str(run_dir / "service.sock")
    mock_ws = MockWsProcess(service_sock)
    mock_ws.start()

    # Start gateway -- override HOME so it uses our short tmp path
    gw = GatewayInstance(uds_path=service_sock)
    # Patch tmp_dir to use our short path so runtime files go there
    gw.tmp_dir = tmp_dir
    gw.start()

    yield gw, mock_ws, tmp_dir

    gw.stop()
    mock_ws.stop()


class TestStreamTunnel:
    def test_ws_connect_and_echo_text(self, ws_env):
        """Connect to /vms/{id}/stream and negotiate the stream subprotocol."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/vms/ws-vm/stream"
            headers = {"Authorization": f"Bearer {gw.token}"}
            async with websockets.connect(
                url, additional_headers=headers, subprotocols=[STREAM_SUBPROTOCOL]
            ) as ws:
                assert ws.subprotocol == STREAM_SUBPROTOCOL
                await ws.send(b"\x00hello from test")
                reply = await asyncio.wait_for(ws.recv(), timeout=5)
                assert reply == b"\x00hello from test"

        asyncio.run(run())

    def test_ws_echo_binary(self, ws_env):
        """Binary messages are relayed correctly."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/vms/ws-vm/stream"
            headers = {"Authorization": f"Bearer {gw.token}"}
            async with websockets.connect(
                url, additional_headers=headers, subprotocols=[STREAM_SUBPROTOCOL]
            ) as ws:
                data = bytes(range(256))
                await ws.send(data)
                reply = await asyncio.wait_for(ws.recv(), timeout=5)
                assert reply == data

        asyncio.run(run())

    def test_ws_multiple_messages(self, ws_env):
        """Multiple messages round-trip correctly."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/vms/ws-vm/stream"
            headers = {"Authorization": f"Bearer {gw.token}"}
            async with websockets.connect(
                url, additional_headers=headers, subprotocols=[STREAM_SUBPROTOCOL]
            ) as ws:
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
            url = f"ws://127.0.0.1:{gw.port}/vms/ws-vm/stream"
            headers = {"Authorization": f"Bearer {gw.token}"}
            ws = await websockets.connect(
                url, additional_headers=headers, subprotocols=[STREAM_SUBPROTOCOL]
            )
            await ws.send("before close")
            reply = await asyncio.wait_for(ws.recv(), timeout=5)
            assert reply == "before close"
            await ws.close()

        asyncio.run(run())

    def test_ws_invalid_id_rejected(self, ws_env):
        """WebSocket upgrade fails for invalid VM ID (dots)."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/vms/vm..bad/stream"
            headers = {"Authorization": f"Bearer {gw.token}"}
            with pytest.raises(websockets.exceptions.InvalidStatus):
                await websockets.connect(
                    url, additional_headers=headers, subprotocols=[STREAM_SUBPROTOCOL]
                )

        asyncio.run(run())

    def test_ws_no_auth_rejected(self, ws_env):
        """WebSocket without auth token is rejected with 401."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/vms/ws-vm/stream"
            with pytest.raises(websockets.exceptions.InvalidStatus):
                await websockets.connect(url)

        asyncio.run(run())

    def test_ws_unknown_vm_relays_the_service_refusal(self, ws_env):
        """The service's 404 for an unknown VM reaches the client unchanged."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/vms/no-such-vm/stream"
            headers = {"Authorization": f"Bearer {gw.token}"}
            with pytest.raises(websockets.exceptions.InvalidStatus) as refused:
                await websockets.connect(
                    url, additional_headers=headers, subprotocols=[STREAM_SUBPROTOCOL]
                )
            assert refused.value.response.status_code == HTTPStatus.NOT_FOUND

        asyncio.run(run())

    def test_retired_terminal_route_is_gone(self, ws_env):
        """`/terminal/{id}` no longer exists, even with a valid token."""
        gw, _, _ = ws_env

        async def run():
            url = f"ws://127.0.0.1:{gw.port}/terminal/ws-vm?token={gw.token}"
            with pytest.raises(websockets.exceptions.InvalidStatus) as refused:
                await websockets.connect(url)
            assert refused.value.response.status_code in (
                HTTPStatus.UNAUTHORIZED,
                HTTPStatus.NOT_FOUND,
            )

        asyncio.run(run())


def test_mock_ws_process_stop_does_not_leak_thread_exception():
    """MockWsProcess.stop() must shut the loop down cleanly.

    Regression test for a teardown race where stop() called loop.stop()
    while the daemon thread was inside run_until_complete(serve_forever()),
    which raises "Event loop stopped before Future completed." on the
    worker thread. That leak surfaced as PytestUnhandledThreadExceptionWarning.
    """
    captured: list[threading.ExceptHookArgs] = []
    original_hook = threading.excepthook

    def hook(args: threading.ExceptHookArgs) -> None:
        captured.append(args)

    threading.excepthook = hook
    tmp_dir = Path(tempfile.mkdtemp(prefix="capsem-mockws-", dir="/tmp"))
    try:
        sock_path = str(tmp_dir / "ws.sock")
        proc = MockWsProcess(sock_path)
        proc.start()
        proc.stop()
        # Allow any late-arriving exception to be delivered to the hook.
        time.sleep(0.1)
        assert proc._thread is not None and not proc._thread.is_alive(), (
            "worker thread did not exit after stop()"
        )
        assert not captured, (
            "MockWsProcess worker thread raised: "
            f"{captured[0].exc_type.__name__}: {captured[0].exc_value}"
        )
    finally:
        threading.excepthook = original_hook
        import shutil

        shutil.rmtree(tmp_dir, ignore_errors=True)
