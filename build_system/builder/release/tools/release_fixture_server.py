#!/usr/bin/env python3
"""Release-owned exact-byte loopback transport for transition fixtures."""

from __future__ import annotations

import contextlib
import http.server
import socket
import socketserver
import threading
from collections.abc import Iterator
from pathlib import Path


class ExactReleaseHandler(http.server.SimpleHTTPRequestHandler):
    """Serve current file bytes and never validate a stale conditional hit."""

    root: Path

    def __init__(
        self,
        request: socket.socket | tuple[bytes, socket.socket],
        client_address: tuple[str, int],
        server: socketserver.BaseServer,
    ) -> None:
        super().__init__(request, client_address, server, directory=str(self.root))

    def translate_path(self, path: str) -> str:
        candidate = Path(super().translate_path(path)).resolve()
        if not candidate.is_relative_to(self.root):
            return str(self.root / ".capsem-forbidden")
        return str(candidate)

    def send_head(self):
        # SimpleHTTPRequestHandler otherwise returns 304 from a one-second
        # Last-Modified timestamp after an atomic same-second promotion.
        for header in ("If-Modified-Since", "If-None-Match"):
            if header in self.headers:
                del self.headers[header]
        return super().send_head()

    def end_headers(self) -> None:
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, format: str, *args: object) -> None:
        return


def handler_for_root(root: Path) -> type[ExactReleaseHandler]:
    resolved = root.resolve()
    return type(
        "RootedExactReleaseHandler",
        (ExactReleaseHandler,),
        {"root": resolved},
    )


@contextlib.contextmanager
def serve_release_root(root: Path) -> Iterator[str]:
    """Serve one existing root on an ephemeral loopback port."""

    resolved = root.resolve()
    if not resolved.is_dir():
        raise ValueError(f"release fixture root must be a directory: {resolved}")
    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0),
        handler_for_root(resolved),
    )
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        host, port = server.server_address[:2]
        yield f"http://{host}:{port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
