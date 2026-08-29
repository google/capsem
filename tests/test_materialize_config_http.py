"""Black-box HTTP tests for the shared runtime-config materializer."""

from __future__ import annotations

import http.server
import json
import os
import subprocess
import threading
from pathlib import Path

import pytest
from capsem_builder.release.tools import (
    build_complete_release_channel,
    local_release_glowup,
)

PROJECT_ROOT = Path(__file__).resolve().parent.parent
MATERIALIZER = PROJECT_ROOT / "build_system" / "scripts" / "build" / "materialize-config.sh"
EXPECTED_USER_AGENT = "capsem-materialize-config/1"
EXPECTED_RELEASE_USER_AGENT = "capsem-release-client/1"


def _fake_materializer_repo(tmp_path: Path) -> tuple[Path, Path]:
    repo = tmp_path / "repo"
    for profile_id in ("code", "co-work"):
        profile_root = repo / "config" / "profiles" / profile_id
        profile_root.mkdir(parents=True)
        (profile_root / "profile.toml").write_text(f'id = "{profile_id}"\n')
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    fake_cargo = fake_bin / "cargo"
    fake_cargo.write_text(
        "#!/bin/sh\n"
        'if [ -n "${FAKE_CARGO_LOG:-}" ]; then\n'
        '  printf "%s\\n" "$*" >> "$FAKE_CARGO_LOG"\n'
        "fi\n"
        "exit 0\n"
    )
    fake_cargo.chmod(0o755)
    return repo, fake_bin


def _run_materializer(
    tmp_path: Path,
    manifest: dict[str, object],
    *,
    arch: str = "arm64",
    cargo_log: Path | None = None,
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
    if cargo_log is not None:
        env["FAKE_CARGO_LOG"] = str(cargo_log)
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
            "code": {"architectures": [{"architecture": "arm64", "images": [], "config": []}]}
        },
        "packages": [],
        "status": "current",
        "version": "1.5.test",
    }

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:
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

        def log_message(self, format: str, *_args: object) -> None:
            _ = format

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
                "CAPSEM_ASSET_MANIFEST": (f"http://127.0.0.1:{server.server_port}/manifest.json"),
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

    assert 'Request(source, headers={"User-Agent": USER_AGENT})' in source
    assert "urlopen(request, timeout=60)" in source
    assert "urlopen(source, timeout=60)" not in source
    assert 'elif "profiles" in manifest:' in source


def test_materializer_accepts_legacy_asset_manifest(tmp_path: Path) -> None:
    result = _run_materializer(
        tmp_path,
        {
            "assets": {
                "current": "2.0.0",
                "releases": {"2.0.0": {"arches": {"arm64": {}}}},
            }
        },
    )

    assert result.returncode == 0, result.stderr


def test_materializer_accepts_release_graph_manifest(tmp_path: Path) -> None:
    cargo_log = tmp_path / "cargo.log"
    result = _run_materializer(
        tmp_path,
        {
            "channel": "stable",
            "profiles": {
                "code": {"architectures": [{"architecture": "arm64"}]},
            },
            "packages": [],
        },
        cargo_log=cargo_log,
    )

    assert result.returncode == 0, result.stderr
    calls = cargo_log.read_text().splitlines()
    assert len(calls) == 1
    assert "/profiles/code/profile.toml" in calls[0]
    assert "co-work" not in calls[0]


def test_materializer_uses_every_active_manifest_member_and_skips_revoked(
    tmp_path: Path,
) -> None:
    cargo_log = tmp_path / "cargo.log"
    result = _run_materializer(
        tmp_path,
        {
            "channel": "nightly",
            "profiles": {
                "code": {"architectures": [{"architecture": "arm64"}]},
                "co-work": {
                    "status": "revoked",
                    "architectures": [{"architecture": "arm64"}],
                },
            },
            "packages": [],
        },
        cargo_log=cargo_log,
    )

    assert result.returncode == 0, result.stderr
    calls = cargo_log.read_text().splitlines()
    assert len(calls) == 1
    assert "/profiles/code/profile.toml" in calls[0]
    assert "co-work" not in calls[0]


def test_materializer_rejects_missing_selected_profile_before_cargo(
    tmp_path: Path,
) -> None:
    cargo_log = tmp_path / "cargo.log"
    result = _run_materializer(
        tmp_path,
        {
            "channel": "nightly",
            "profiles": {
                "experimental": {"architectures": [{"architecture": "arm64"}]},
            },
            "packages": [],
        },
        cargo_log=cargo_log,
    )

    assert result.returncode != 0
    assert "selected release profile source is missing" in result.stderr
    assert "experimental/profile.toml" in result.stderr
    assert not cargo_log.exists(), "membership mismatch must fail before compiling capsem-admin"


def test_materializer_custom_output_preserves_shared_default_config(
    tmp_path: Path,
) -> None:
    repo, fake_bin = _fake_materializer_repo(tmp_path)
    shared_marker = repo / "target" / "config" / "profiles" / "shared.marker"
    shared_marker.parent.mkdir(parents=True)
    shared_marker.write_text("functional cohort")
    isolated_output = repo / "target" / "install-test-config"
    isolated_marker = isolated_output / "stale.marker"
    isolated_output.mkdir(parents=True)
    isolated_marker.write_text("replace me")
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                "channel": "stable",
                "profiles": {
                    "code": {"architectures": [{"architecture": "arm64"}]},
                },
                "packages": [],
            }
        )
    )
    env = os.environ.copy()
    env.update(
        {
            "CAPSEM_REPO_ROOT": str(repo),
            "CAPSEM_ARCH": "arm64",
            "CAPSEM_ASSET_MANIFEST": str(manifest_path),
            "CAPSEM_CONFIG_OUTPUT_ROOT": str(isolated_output),
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

    assert result.returncode == 0, result.stderr
    assert shared_marker.read_text() == "functional cohort"
    assert not isolated_marker.exists()


def test_materializer_finalizes_one_exact_runtime_content_pair(tmp_path: Path) -> None:
    repo, fake_bin = _fake_materializer_repo(tmp_path)
    pair = repo / "target" / "paired-content"
    assets = pair / "assets"
    config = pair / "config"
    inputs = pair / "inputs"
    assets.mkdir(parents=True)
    inputs.mkdir(parents=True)
    release_graph = {
        "channel": "stable",
        "profiles": {"code": {"architectures": [{"architecture": "arm64"}]}},
        "packages": [],
    }
    original = json.dumps(release_graph, sort_keys=True).encode()
    (assets / "manifest.json").write_bytes(original)
    (inputs / "manifest.json").write_bytes(original)
    runtime_projection = (
        b'{"assets":{"current":"runtime","releases":{"runtime":{"arches":{"arm64":{}}}}}}'
    )
    fake_cargo = fake_bin / "cargo"
    fake_cargo.write_text(
        "#!/bin/sh\n"
        "output=\n"
        'while [ "$#" -gt 0 ]; do\n'
        '  if [ "$1" = --output-root ]; then output=$2; shift 2; else shift; fi\n'
        "done\n"
        'mkdir -p "$output/assets"\n'
        'printf \'%s\' "$CAPSEM_FAKE_RUNTIME_MANIFEST" > "$output/assets/manifest.json"\n',
        encoding="utf-8",
    )
    fake_cargo.chmod(0o755)
    env = {
        **os.environ,
        "CAPSEM_REPO_ROOT": str(repo),
        "CAPSEM_ARCH": "arm64",
        "CAPSEM_ASSET_MANIFEST": str(assets / "manifest.json"),
        "CAPSEM_ASSETS_PATH": str(assets),
        "CAPSEM_CONFIG_OUTPUT_ROOT": str(config),
        "CAPSEM_FAKE_RUNTIME_MANIFEST": runtime_projection.decode(),
        "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
    }

    result = subprocess.run(
        ["bash", str(MATERIALIZER), "--pair-content"],
        env=env,
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert (assets / "manifest.json").read_bytes() == runtime_projection
    assert (config / "assets" / "manifest.json").read_bytes() == runtime_projection
    assert (inputs / "manifest.json").read_bytes() == original


def test_materializer_refuses_unknown_pairing_mode_before_cargo(tmp_path: Path) -> None:
    repo, fake_bin = _fake_materializer_repo(tmp_path)
    cargo_log = tmp_path / "cargo.log"

    result = subprocess.run(
        ["bash", str(MATERIALIZER), "--best-effort-pair"],
        env={
            **os.environ,
            "CAPSEM_REPO_ROOT": str(repo),
            "FAKE_CARGO_LOG": str(cargo_log),
            "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
        },
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )

    assert result.returncode == 2
    assert "unknown materialize-config argument" in result.stderr
    assert not cargo_log.exists()


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


def test_public_release_readers_identify_capsem_to_http_edge() -> None:
    observed_user_agents: list[str] = []
    manifest = {"channel": "stable", "profiles": {}, "packages": []}

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
        PROJECT_ROOT / "build_system/scripts/build/materialize-config.sh": ("urlopen(source",),
        Path(build_complete_release_channel.__file__): ("urlopen(source",),
        Path(local_release_glowup.__file__): ("urlopen(manifest_url",),
    }

    for path, forbidden_calls in readers.items():
        source = path.read_text()
        for forbidden_call in forbidden_calls:
            assert forbidden_call not in source, f"{path} uses {forbidden_call}"
