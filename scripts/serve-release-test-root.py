#!/usr/bin/env python3
"""Serve one generated release-test directory on an ephemeral loopback port."""

from __future__ import annotations

import argparse
import http.server
import json
import os
import signal
import threading
from pathlib import Path

try:
    from release_fixture_server import handler_for_root
except ModuleNotFoundError:
    from scripts.release_fixture_server import handler_for_root


def _write_ready(path: Path, root: Path, port: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(
        json.dumps(
            {
                "base_url": f"http://127.0.0.1:{port}",
                "root": str(root),
            },
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--ready-file", required=True, type=Path)
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()

    root = args.root.resolve()
    if not root.is_dir():
        parser.error(f"--root must be an existing directory: {root}")
    ready = args.ready_file.resolve()
    handler = handler_for_root(root)
    if not 0 <= args.port <= 65535:
        parser.error("--port must be between 0 and 65535")
    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), handler)
    server.daemon_threads = True

    def stop_server(_signum: int, _frame: object) -> None:
        threading.Thread(target=server.shutdown, daemon=True).start()

    signal.signal(signal.SIGTERM, stop_server)
    signal.signal(signal.SIGINT, stop_server)
    _write_ready(ready, root, server.server_address[1])
    try:
        server.serve_forever()
    finally:
        server.server_close()
        ready.unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
