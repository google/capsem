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


class _LoopbackReleaseHandler(http.server.SimpleHTTPRequestHandler):
    root: Path

    def __init__(self, *args: object, **kwargs: object) -> None:
        super().__init__(*args, directory=str(self.root), **kwargs)

    def translate_path(self, path: str) -> str:
        candidate = Path(super().translate_path(path)).resolve()
        if not candidate.is_relative_to(self.root):
            return str(self.root / ".capsem-forbidden")
        return str(candidate)

    def log_message(self, format: str, *args: object) -> None:
        return


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
    args = parser.parse_args()

    root = args.root.resolve()
    if not root.is_dir():
        parser.error(f"--root must be an existing directory: {root}")
    ready = args.ready_file.resolve()
    handler = type(
        "LoopbackReleaseHandler",
        (_LoopbackReleaseHandler,),
        {"root": root},
    )
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
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
