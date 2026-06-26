"""Route matrix for profile-owned service API surfaces.

The UI and TUI must be able to build profile pages from explicit profile
routes. A missing route, fallback route, 404, or 501 is a product bug.
"""

from __future__ import annotations

import json
import subprocess
from typing import Any

from helpers.route_matrix import RouteSpec, assert_profile_route_matrix


PROFILES = ("code", "co-work")


def _uds_request(client: Any, spec: RouteSpec) -> Any:
    cmd = [
        "curl",
        "-s",
        "-S",
        "--unix-socket",
        client.socket_path,
        "-X",
        spec.method,
        "-H",
        "Content-Type: application/json",
        "-w",
        "\n%{http_code}",
        "--max-time",
        "30",
        f"http://localhost{spec.path}",
    ]
    if spec.body is not None:
        cmd.extend(["-d", json.dumps(spec.body)])
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=35)
    assert result.returncode == 0, (spec.path, result.stderr)
    body, _, status_text = result.stdout.rpartition("\n")
    assert status_text == "200", (spec.path, status_text, body)
    return json.loads(body)


def test_profile_route_matrix_exists_for_every_ui_profile(client: Any) -> None:
    listed = client.get("/profiles/list")
    listed_ids = {profile["id"] for profile in listed["profiles"]}
    assert set(PROFILES) <= listed_ids

    assert_profile_route_matrix(
        profiles=PROFILES,
        request=lambda spec: _uds_request(client, spec),
    )
