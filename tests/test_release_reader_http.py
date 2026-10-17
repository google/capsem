"""Black-box HTTP tests for the public release-graph readers."""

from __future__ import annotations

import http.server
import json
import threading
from pathlib import Path

from capsem_builder.release.tools import (
    build_complete_release_channel,
    local_release_glowup,
)

EXPECTED_RELEASE_USER_AGENT = "capsem-release-client/1"


def test_public_release_readers_identify_capsem_to_http_edge() -> None:
    observed_user_agents: list[str] = []
    manifest = {"channel": "stable", "runtime": {}, "packages": []}

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            user_agent = self.headers.get("User-Agent", "")
            observed_user_agents.append(user_agent)
            if user_agent != EXPECTED_RELEASE_USER_AGENT:
                self.send_response(403)
                self.end_headers()
                return
            body = json.dumps(manifest).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *_args: object) -> None:
            _ = format

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        value = build_complete_release_channel.read_json_source(
            f"http://127.0.0.1:{server.server_port}/manifest.json"
        )
    finally:
        server.shutdown()
        thread.join(timeout=5)
        server.server_close()

    assert value == manifest
    assert observed_user_agents == [EXPECTED_RELEASE_USER_AGENT]


def test_public_release_readers_never_pass_a_url_string_to_urlopen() -> None:
    readers = {
        Path(build_complete_release_channel.__file__): ("urlopen(source",),
        Path(local_release_glowup.__file__): ("urlopen(manifest_url",),
    }

    for path, forbidden_calls in readers.items():
        source = path.read_text()
        for forbidden_call in forbidden_calls:
            assert forbidden_call not in source, f"{path} uses {forbidden_call}"
