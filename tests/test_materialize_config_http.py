"""Black-box HTTP tests for the shared runtime-config materializer."""

from __future__ import annotations

import http.server
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import threading

import pytest


PROJECT_ROOT = Path(__file__).resolve().parent.parent
MATERIALIZER = PROJECT_ROOT / "scripts" / "materialize-config.sh"
EXPECTED_USER_AGENT = "capsem-materialize-config/1"
EXPECTED_RELEASE_USER_AGENT = "capsem-release-client/1"


def _fake_materializer_repo(tmp_path: Path) -> tuple[Path, Path]:
    repo = tmp_path / "repo"
    (repo / "config" / "profiles" / "code").mkdir(parents=True)
    (repo / "config" / "profiles" / "code" / "profile.toml").write_text(
        'id = "code"\n'
    )
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    fake_cargo = fake_bin / "cargo"
    fake_cargo.write_text("#!/bin/sh\nexit 0\n")
    fake_cargo.chmod(0o755)
    return repo, fake_bin


def _run_materializer(
    tmp_path: Path,
    manifest: dict[str, object],
    *,
    arch: str = "arm64",
) -> subprocess.CompletedProcess[str]:
    repo, fake_bin = _fake_materializer_repo(tmp_path)
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(json.dumps(manifest))
    env = os.environ.copy()
    env.update(
        {
            "CAPSEM_REPO_ROOT": str(repo),
            "CAPSEM_ARCH": arch,
            "CAPSEM_ASSET_MANIFEST": str(manifest_path),
            "PATH": f"{fake_bin}{os.pathsep}{env['PATH']}",
        }
    )
    return subprocess.run(
        ["bash", str(MATERIALIZER)],
        env=env,
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )


def test_materializer_http_request_identifies_capsem(tmp_path: Path) -> None:
    """A CDN may reject Python's default user agent; the release path must not use it."""

    observed_user_agents: list[str] = []
    manifest = {
        "channel": "stable",
        "profiles": {
            "code": {
                "architectures": [
                    {"architecture": "arm64", "images": [], "config": []}
                ]
            }
        },
        "packages": [],
        "status": "current",
        "version": "1.5.test",
    }

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802 - stdlib handler API
            user_agent = self.headers.get("User-Agent", "")
            observed_user_agents.append(user_agent)
            if user_agent != EXPECTED_USER_AGENT:
                self.send_response(403)
                self.end_headers()
                return
            body = json.dumps(manifest).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, _format: str, *_args: object) -> None:
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        repo, fake_bin = _fake_materializer_repo(tmp_path)

        env = os.environ.copy()
        env.update(
            {
                "CAPSEM_REPO_ROOT": str(repo),
                "CAPSEM_ARCH": "arm64",
                "CAPSEM_ASSET_MANIFEST": (
                    f"http://127.0.0.1:{server.server_port}/manifest.json"
                ),
                "PATH": f"{fake_bin}{os.pathsep}{env['PATH']}",
            }
        )
        result = subprocess.run(
            ["bash", str(MATERIALIZER)],
            env=env,
            text=True,
            capture_output=True,
            timeout=30,
            check=False,
        )
    finally:
        server.shutdown()
        thread.join(timeout=5)
        server.server_close()

    assert result.returncode == 0, result.stderr
    assert observed_user_agents == [EXPECTED_USER_AGENT]


def test_materializer_never_uses_bare_urlopen_for_http_manifest() -> None:
    source = MATERIALIZER.read_text()

    assert "Request(source, headers={\"User-Agent\": USER_AGENT})" in source
    assert "urlopen(request, timeout=60)" in source
    assert "urlopen(source, timeout=60)" not in source
    assert 'elif "profiles" in manifest:' in source


def test_materializer_accepts_legacy_asset_manifest(tmp_path: Path) -> None:
    result = _run_materializer(
        tmp_path,
        {
            "assets": {
                "current": "2030.01.01.1",
                "releases": {"2030.01.01.1": {"arches": {"arm64": {}}}},
            }
        },
    )

    assert result.returncode == 0, result.stderr


def test_materializer_accepts_release_graph_manifest(tmp_path: Path) -> None:
    result = _run_materializer(
        tmp_path,
        {
            "channel": "stable",
            "profiles": {
                "code": {"architectures": [{"architecture": "arm64"}]},
            },
            "packages": [],
        },
    )

    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize(
    ("manifest", "message"),
    [
        ({}, "manifest must contain legacy assets or release profiles"),
        ({"profiles": {}}, "release manifest profiles must be a non-empty object"),
        (
            {"profiles": {"code": {"architectures": []}}},
            "release manifest profiles contain no architectures",
        ),
    ],
)
def test_materializer_rejects_incomplete_manifest_schemas(
    tmp_path: Path,
    manifest: dict[str, object],
    message: str,
) -> None:
    result = _run_materializer(tmp_path, manifest)

    assert result.returncode != 0
    assert message in result.stderr


def _load_script(path: str):
    script = PROJECT_ROOT / path
    spec = importlib.util.spec_from_file_location(script.stem.replace("-", "_"), script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.mark.parametrize(
    ("path", "reader_name"),
    [
        ("scripts/check-asset-release-delta.py", "_load_url"),
        ("scripts/build-complete-release-channel.py", "read_json_source"),
    ],
)
def test_public_release_readers_identify_capsem_to_http_edge(
    tmp_path: Path,
    path: str,
    reader_name: str,
) -> None:
    observed_user_agents: list[str] = []
    manifest = {"channel": "stable", "profiles": {}, "packages": []}

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802 - stdlib handler API
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

        def log_message(self, _format: str, *_args: object) -> None:
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        module = _load_script(path)
        value = getattr(module, reader_name)(
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
        "scripts/materialize-config.sh": ("urlopen(source",),
        "scripts/check-asset-release-delta.py": ("urlopen(url",),
        "scripts/build-complete-release-channel.py": ("urlopen(source",),
        "scripts/local-release-glowup.py": ("urlopen(manifest_url",),
    }

    for path, forbidden_calls in readers.items():
        source = (PROJECT_ROOT / path).read_text()
        for forbidden_call in forbidden_calls:
            assert forbidden_call not in source, f"{path} uses {forbidden_call}"
