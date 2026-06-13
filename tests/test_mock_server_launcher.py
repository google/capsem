from __future__ import annotations

import socket
import threading
import time

from helpers.mock_server import start_mock_server, stop_process


def test_mock_server_launcher_waits_for_busy_address_then_starts() -> None:
    holder = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    holder.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    holder.bind(("127.0.0.1", 0))
    holder.listen(1)
    host, port = holder.getsockname()
    addr = f"{host}:{port}"

    def release_holder() -> None:
        time.sleep(0.3)
        holder.close()

    threading.Thread(target=release_holder, daemon=True).start()
    proc = None
    try:
        proc, ready = start_mock_server(addr=addr, timeout_s=5, retry_interval_s=0.05)
        assert ready["service"] == "capsem-mock-server"
        assert ready["base_url"] == f"http://{addr}"
    finally:
        stop_process(proc)
