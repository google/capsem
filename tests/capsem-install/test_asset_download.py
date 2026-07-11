"""End-to-end test for `capsem update --assets` against a local HTTP fixture.

Validates the asset download path wired through
`capsem_core::asset_manager::download_missing_assets`:

  - happy path: files land at `<CAPSEM_HOME>/assets/<arch>/<hash-filename>`
    with matching blake3 and 0o444 perms
  - hash mismatch: server serves wrong bytes -> command fails, no file left
  - 404: asset URL missing a file -> command fails with URL in error

The server is a threaded `http.server` bound to 127.0.0.1:0. The test writes a
minimal v2 manifest whose hashes match the fixture bytes, then runs the CLI
against `CAPSEM_HOME` + `CAPSEM_ASSET_BASE_URL` pointed at the server.
"""

from __future__ import annotations

import json
import hashlib
import os
import platform
import subprocess
import threading
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

from .conftest import INSTALL_DIR

# The installed layout mounts a proper Linux ELF at INSTALL_DIR/capsem via
# dpkg (in the Docker install harness) or simulate-install.sh (local dev).
# Hardcoding `target/debug/capsem` crashed the Docker test harness with
# `Exec format error` because $PWD/target/debug/capsem is the macOS host
# build, not the Linux binary -- CARGO_TARGET_DIR=/cargo-target inside the
# container never lands binaries under /src/target.
CAPSEM_BIN = INSTALL_DIR / "capsem"
REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _arch() -> str:
    m = platform.machine().lower()
    if m in ("arm64", "aarch64"):
        return "arm64"
    return "x86_64"


ASSET_VERSION = "2030.0101.1"
NEW_ASSET_VERSION = "2030.0101.2"


def _blake3(data: bytes) -> str:
    # Prefer the `blake3` module if present (capsem uses it), fall back to
    # shelling out to `b3sum` which is a dev-env requirement via `just doctor`.
    try:
        import blake3 as b3  # type: ignore

        return b3.blake3(data).hexdigest()
    except ImportError:
        pass
    proc = subprocess.run(
        ["b3sum", "--no-names"],
        input=data,
        capture_output=True,
        check=True,
    )
    return proc.stdout.decode().strip().split()[0]


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _read_update_log(capsem_home: Path) -> list[dict]:
    path = capsem_home / "logs" / "update.log"
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]


def _make_manifest(arch: str, files: dict[str, bytes], asset_version: str = ASSET_VERSION) -> dict:
    """Build a minimal v2 manifest for the given arch + byte blobs."""
    return {
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": asset_version,
            "releases": {
                asset_version: {
                    "date": "2030-01-01",
                    "deprecated": False,
                    "min_binary": "1.0.0",
                    "arches": {
                        arch: {
                            name: {"hash": _blake3(blob), "size": len(blob)}
                            for name, blob in files.items()
                        }
                    },
                }
            },
        },
        "binaries": {
            "current": "1.0.1",
            "releases": {
                "1.0.1": {
                    "date": "2030-01-01",
                    "deprecated": False,
                    "min_assets": asset_version,
                }
            },
        },
    }


def _make_release_channel_manifest(
    arch: str,
    files: dict[str, bytes],
    *,
    profile_id: str = "default",
    revision: str = NEW_ASSET_VERSION,
) -> dict:
    """Build the split-lane release graph shape published at /assets/<channel>/manifest.json."""
    return {
        "version": "1.5.2030010102",
        "status": "current",
        "packages": [],
        "profiles": {
            profile_id: {
                "version": "1",
                "id": profile_id,
                "name": "Default",
                "revision": revision,
                "status": "current",
                "architectures": [
                    {
                        "architecture": arch,
                        "software": [],
                        "config": [],
                        "images": [
                            {
                                "kind": kind,
                                "name": name,
                                "url": f"/profiles/releases/{revision}/{profile_id}/{arch}/{name}",
                                "bytes": len(blob),
                                "digest": {
                                    "sha256": _sha256(blob),
                                    "blake3": _blake3(blob),
                                },
                                "status": "current",
                            }
                            for kind, name, blob in (
                                ("kernel", "vmlinuz", files["vmlinuz"]),
                                ("initrd", "initrd.img", files["initrd.img"]),
                                ("rootfs", "rootfs.erofs", files["rootfs.erofs"]),
                            )
                        ],
                        "evidence": [],
                    }
                ],
            }
        },
    }


def _hashed_asset_name(logical_name: str, blob: bytes) -> str:
    prefix = _blake3(blob)[:16]
    if "." in logical_name:
        stem, ext = logical_name.split(".", 1)
        return f"{stem}-{prefix}.{ext}"
    return f"{logical_name}-{prefix}"


def _write_installed_manifest_and_assets(
    assets_dir: Path,
    arch: str,
    files: dict[str, bytes],
    *,
    asset_version: str,
    origin: dict,
) -> dict:
    arch_assets = assets_dir / arch
    arch_assets.mkdir(parents=True)
    manifest = _make_manifest(arch, files, asset_version)
    (assets_dir / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    (assets_dir / "manifest-origin.json").write_text(
        json.dumps(origin, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    for name, blob in files.items():
        target = arch_assets / _hashed_asset_name(name, blob)
        target.write_bytes(blob)
        target.chmod(0o444)
    return manifest


@pytest.fixture
def http_fixture(tmp_path: Path):
    """Spin an http.server in the background; yield (base_url, serve_dir)."""
    serve_dir = tmp_path / "release"
    serve_dir.mkdir()

    serve_str = str(serve_dir)
    requested_paths: list[str] = []

    class Handler(SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            kwargs["directory"] = serve_str
            super().__init__(*args, **kwargs)

        def do_GET(self):
            requested_paths.append(self.path.split("?", maxsplit=1)[0])
            super().do_GET()

        def log_message(self, format, *args):
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    host, port = server.server_address
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield (f"http://{host}:{port}", serve_dir, requested_paths)
    finally:
        server.shutdown()
        server.server_close()


def _run(env: dict, *args: str) -> subprocess.CompletedProcess:
    assert CAPSEM_BIN.exists(), (
        f"capsem binary not built at {CAPSEM_BIN}; run `cargo build -p capsem` first"
    )
    return subprocess.run(
        [str(CAPSEM_BIN), *args],
        capture_output=True,
        text=True,
        timeout=30,
        env={**os.environ, **env},
    )


def _fresh_capsem_binary() -> Path:
    bin_src = Path(os.environ.get("CAPSEM_BIN_SRC", REPO_ROOT / "target" / "debug"))
    binary = bin_src / "capsem"
    source_paths = [
        REPO_ROOT / "crates" / "capsem" / "src" / "update.rs",
        REPO_ROOT / "crates" / "capsem" / "src" / "main.rs",
    ]
    if binary.is_file() and all(
        binary.stat().st_mtime >= path.stat().st_mtime for path in source_paths
    ):
        return binary
    result = subprocess.run(
        ["cargo", "build", "-p", "capsem"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        timeout=120,
        env={**os.environ, "CARGO_TARGET_DIR": str(bin_src.parent)},
    )
    assert result.returncode == 0, (
        f"cargo build -p capsem failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    assert binary.is_file()
    return binary


def _run_binary(binary: Path, env: dict, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(binary), *args],
        capture_output=True,
        text=True,
        timeout=30,
        env={**os.environ, **env},
    )


@contextmanager
def _serve_interrupted_asset_channel(
    manifest: dict,
    blobs: dict[str, bytes],
    interrupted_path: str,
):
    payloads = {
        "/assets/stable/manifest.json": json.dumps(manifest).encode("utf-8"),
        **blobs,
    }

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            path = self.path.split("?", maxsplit=1)[0]
            payload = payloads.get(path)
            if payload is None:
                self.send_error(404)
                return
            content_type = (
                "application/json" if path.endswith(".json") else "application/octet-stream"
            )
            if path == interrupted_path:
                self.send_response(200)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(payload) + 32))
                self.end_headers()
                self.wfile.write(payload[: max(1, len(payload) // 2)])
                self.wfile.flush()
                self.close_connection = True
                return
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, format, *args):
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()


@contextmanager
def _serve_flaky_asset_channel(manifest: dict, blobs: dict[str, bytes], flaky_path: str):
    payloads = {
        "/assets/stable/manifest.json": json.dumps(manifest).encode("utf-8"),
        **blobs,
    }
    requested_paths: list[str] = []
    failures_left = {flaky_path: 1}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            path = self.path.split("?", maxsplit=1)[0]
            requested_paths.append(path)
            if failures_left.get(path, 0) > 0:
                failures_left[path] -= 1
                self.close_connection = True
                return
            payload = payloads.get(path)
            if payload is None:
                self.send_error(404)
                return
            content_type = (
                "application/json" if path.endswith(".json") else "application/octet-stream"
            )
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, format, *args):
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}", requested_paths
    finally:
        server.shutdown()
        server.server_close()


def test_update_assets_downloads_missing(tmp_path: Path, http_fixture, installed_layout):
    base_url, serve_dir, _requested_paths = http_fixture
    arch = _arch()

    # Fixture bytes: small so hashing is cheap but non-empty.
    files = {
        "vmlinuz": b"test-kernel-bytes-" + os.urandom(64),
        "initrd.img": b"test-initrd-bytes-" + os.urandom(64),
        "rootfs.erofs": b"test-rootfs-bytes-" + os.urandom(64),
    }

    release_dir = serve_dir / ASSET_VERSION
    release_dir.mkdir()
    for name, blob in files.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)

    manifest = _make_manifest(arch, files)
    (assets / "manifest.json").write_text(json.dumps(manifest))

    env = {
        "CAPSEM_HOME": str(capsem_home),
        "CAPSEM_ASSET_BASE_URL": base_url,
    }
    r = _run(env, "update", "--assets")
    assert r.returncode == 0, f"stdout={r.stdout}\nstderr={r.stderr}"

    for name, blob in files.items():
        expected_hash = _blake3(blob)
        prefix = expected_hash[:16]
        if "." in name:
            stem, ext = name.split(".", 1)
            hashed = f"{stem}-{prefix}.{ext}"
        else:
            hashed = f"{name}-{prefix}"
        target = assets / arch / hashed
        assert target.exists(), f"{target} not downloaded. stdout={r.stdout}"
        assert target.read_bytes() == blob, "downloaded bytes differ"
        if hasattr(os, "stat"):
            mode = os.stat(target).st_mode & 0o777
            # 0o444 target (readable by all, writable by none).
            assert mode == 0o444, f"{target} perms {oct(mode)} != 0o444"


def test_update_assets_refreshes_remote_channel_manifest_before_download(
    tmp_path: Path,
    http_fixture,
    installed_layout,
):
    base_url, serve_dir, requested_paths = http_fixture
    arch = _arch()

    old_files = {
        "vmlinuz": b"old-kernel",
        "initrd.img": b"old-initrd",
        "rootfs.erofs": b"old-rootfs",
    }
    new_files = {
        "vmlinuz": b"new-kernel-bytes-" + os.urandom(64),
        "initrd.img": b"new-initrd-bytes-" + os.urandom(64),
        "rootfs.erofs": b"new-rootfs-bytes-" + os.urandom(64),
    }

    channel_manifest_url = f"{base_url}/assets/stable/manifest.json"
    channel_manifest = _make_manifest(arch, new_files, NEW_ASSET_VERSION)
    channel_manifest_path = serve_dir / "assets" / "stable" / "manifest.json"
    channel_manifest_path.parent.mkdir(parents=True)
    channel_manifest_path.write_text(json.dumps(channel_manifest), encoding="utf-8")

    release_dir = serve_dir / "assets" / "releases" / NEW_ASSET_VERSION
    release_dir.mkdir(parents=True)
    for name, blob in new_files.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)
    (assets / "manifest.json").write_text(
        json.dumps(_make_manifest(arch, old_files, "2030.0101.1")),
        encoding="utf-8",
    )
    (assets / "manifest-origin.json").write_text(
        json.dumps(
            {
                "schema": "capsem.manifest_origin.v1",
                "origin": "package",
                "source": channel_manifest_url,
                "packaged_at": "2026-06-16T00:00:00Z",
            },
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

    result = _run({"CAPSEM_HOME": str(capsem_home)}, "update", "--assets")

    assert result.returncode == 0, f"stdout={result.stdout}\nstderr={result.stderr}"
    assert f"Installed asset manifest from {channel_manifest_url}" in result.stdout
    assert "/assets/stable/manifest.json" in requested_paths
    expected_blob_paths = {
        f"/assets/releases/{NEW_ASSET_VERSION}/{arch}-{name}"
        for name in new_files
    }
    assert expected_blob_paths.issubset(set(requested_paths))
    legacy_blob_paths = {f"/{NEW_ASSET_VERSION}/{arch}-{name}" for name in new_files}
    assert set(requested_paths).isdisjoint(legacy_blob_paths), requested_paths
    installed_manifest = json.loads((assets / "manifest.json").read_text())
    assert installed_manifest["assets"]["current"] == NEW_ASSET_VERSION
    origin = json.loads((assets / "manifest-origin.json").read_text())
    assert origin["source"] == channel_manifest_url

    for name, blob in new_files.items():
        expected_hash = _blake3(blob)
        prefix = expected_hash[:16]
        if "." in name:
            stem, ext = name.split(".", 1)
            hashed = f"{stem}-{prefix}.{ext}"
        else:
            hashed = f"{name}-{prefix}"
        target = assets / arch / hashed
        assert target.exists(), f"{target} not downloaded. stdout={result.stdout}"
        assert target.read_bytes() == blob


def test_update_assets_retries_transient_remote_channel_manifest_fetch(
    tmp_path: Path,
    installed_layout,
):
    binary = _fresh_capsem_binary()
    arch = _arch()
    old_files = {
        "vmlinuz": b"old-flaky-manifest-kernel",
        "initrd.img": b"old-flaky-manifest-initrd",
        "rootfs.erofs": b"old-flaky-manifest-rootfs",
    }
    new_files = {
        "vmlinuz": b"new-flaky-manifest-kernel" * 128,
        "initrd.img": b"new-flaky-manifest-initrd" * 128,
        "rootfs.erofs": b"new-flaky-manifest-rootfs" * 128,
    }
    channel_manifest = _make_manifest(arch, new_files, NEW_ASSET_VERSION)
    blobs = {
        f"/assets/releases/{NEW_ASSET_VERSION}/{arch}-{name}": blob
        for name, blob in new_files.items()
    }

    with _serve_flaky_asset_channel(
        channel_manifest, blobs, "/assets/stable/manifest.json"
    ) as (base_url, requested_paths):
        channel_manifest_url = f"{base_url}/assets/stable/manifest.json"
        capsem_home = tmp_path / ".capsem"
        assets = capsem_home / "assets"
        old_origin = {
            "schema": "capsem.manifest_origin.v1",
            "origin": "package",
            "source": channel_manifest_url,
            "packaged_at": "2026-06-16T00:00:00Z",
        }
        _write_installed_manifest_and_assets(
            assets,
            arch,
            old_files,
            asset_version=ASSET_VERSION,
            origin=old_origin,
        )

        result = _run_binary(binary, {"CAPSEM_HOME": str(capsem_home)}, "update", "--assets")

    assert result.returncode == 0, f"stdout={result.stdout}\nstderr={result.stderr}"
    assert requested_paths.count("/assets/stable/manifest.json") >= 2
    assert f"Installed asset manifest from {channel_manifest_url}" in result.stdout
    installed_manifest = json.loads((assets / "manifest.json").read_text())
    assert installed_manifest["assets"]["current"] == NEW_ASSET_VERSION
    for name, blob in new_files.items():
        target = assets / arch / _hashed_asset_name(name, blob)
        assert target.exists(), f"{target} not downloaded. stdout={result.stdout}"
        assert target.read_bytes() == blob


def test_update_assets_records_channel_change_audit_log(
    tmp_path: Path,
    http_fixture,
    installed_layout,
):
    base_url, serve_dir, _requested_paths = http_fixture
    arch = _arch()

    old_files = {
        "vmlinuz": b"audit-old-kernel",
        "initrd.img": b"audit-old-initrd",
        "rootfs.erofs": b"audit-old-rootfs",
    }
    new_files = {
        "vmlinuz": b"audit-new-kernel-" + os.urandom(32),
        "initrd.img": b"audit-new-initrd-" + os.urandom(32),
        "rootfs.erofs": b"audit-new-rootfs-" + os.urandom(32),
    }

    channel_manifest_url = f"{base_url}/assets/nightly/manifest.json"
    channel_manifest = _make_manifest(arch, new_files, NEW_ASSET_VERSION)
    channel_manifest_path = serve_dir / "assets" / "nightly" / "manifest.json"
    channel_manifest_path.parent.mkdir(parents=True)
    channel_manifest_path.write_text(json.dumps(channel_manifest), encoding="utf-8")

    release_dir = serve_dir / "assets" / "releases" / NEW_ASSET_VERSION
    release_dir.mkdir(parents=True)
    for name, blob in new_files.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    _write_installed_manifest_and_assets(
        assets,
        arch,
        old_files,
        asset_version=ASSET_VERSION,
        origin={
            "schema": "capsem.manifest_origin.v1",
            "origin": "package",
            "source": f"{base_url}/assets/stable/manifest.json",
            "package_version": "1.5.100",
            "packaged_at": "2026-06-16T00:00:00Z",
        },
    )
    previous_sha = hashlib.sha256((assets / "manifest.json").read_bytes()).hexdigest()

    result = _run_binary(
        _fresh_capsem_binary(),
        {"CAPSEM_HOME": str(capsem_home)},
        "update",
        "--assets",
        "--manifest",
        channel_manifest_url,
    )

    assert result.returncode == 0, f"stdout={result.stdout}\nstderr={result.stderr}"
    events = _read_update_log(capsem_home)
    complete = events[-1]
    assert complete["schema"] == "capsem.update_audit.v1"
    assert complete["event"] == "asset_update_complete"
    assert complete["action"] == "asset_update"
    assert complete["outcome"] == "success"
    assert complete["source"] == channel_manifest_url
    assert complete["channel"] == "nightly"
    assert complete["previous"]["source"].endswith("/assets/stable/manifest.json")
    assert complete["previous"]["manifest_sha256"] == previous_sha
    assert complete["previous"]["asset_version"] == ASSET_VERSION
    assert complete["previous"]["package_version"] == "1.5.100"
    assert complete["current"]["source"] == channel_manifest_url
    assert complete["current"]["asset_version"] == NEW_ASSET_VERSION
    assert complete["current"]["package_version"] == "1.5.100"
    assert {"source", "manifest_sha256", "asset_version"}.issubset(
        set(complete["changed_fields"])
    )
    assert "package_version" not in complete["changed_fields"]


def test_update_assets_accepts_release_channel_profile_manifest(
    tmp_path: Path,
    http_fixture,
    installed_layout,
):
    base_url, serve_dir, requested_paths = http_fixture
    arch = _arch()

    old_files = {
        "vmlinuz": b"old-profile-graph-kernel",
        "initrd.img": b"old-profile-graph-initrd",
        "rootfs.erofs": b"old-profile-graph-rootfs",
    }
    new_files = {
        "vmlinuz": b"profile-graph-kernel-" + os.urandom(64),
        "initrd.img": b"profile-graph-initrd-" + os.urandom(64),
        "rootfs.erofs": b"profile-graph-rootfs-" + os.urandom(64),
    }

    channel_manifest_url = f"{base_url}/assets/stable/manifest.json"
    channel_manifest = _make_release_channel_manifest(arch, new_files)
    channel_manifest_path = serve_dir / "assets" / "stable" / "manifest.json"
    channel_manifest_path.parent.mkdir(parents=True)
    channel_manifest_path.write_text(json.dumps(channel_manifest), encoding="utf-8")

    release_dir = serve_dir / "profiles" / "releases" / NEW_ASSET_VERSION / "default" / arch
    release_dir.mkdir(parents=True)
    for name, blob in new_files.items():
        (release_dir / name).write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    _write_installed_manifest_and_assets(
        assets,
        arch,
        old_files,
        asset_version=ASSET_VERSION,
        origin={
            "schema": "capsem.manifest_origin.v1",
            "origin": "package",
            "source": channel_manifest_url,
            "packaged_at": "2026-06-16T00:00:00Z",
        },
    )

    result = _run({"CAPSEM_HOME": str(capsem_home)}, "update", "--assets")

    assert result.returncode == 0, f"stdout={result.stdout}\nstderr={result.stderr}"
    assert f"Installed asset manifest from {channel_manifest_url}" in result.stdout
    assert "/assets/stable/manifest.json" in requested_paths
    expected_blob_paths = {
        f"/profiles/releases/{NEW_ASSET_VERSION}/default/{arch}/{name}" for name in new_files
    }
    assert expected_blob_paths.issubset(set(requested_paths))
    assert "missing field `format`" not in result.stderr

    installed_manifest = json.loads((assets / "manifest.json").read_text())
    assert installed_manifest["format"] == 2
    assert installed_manifest["assets"]["current"] == NEW_ASSET_VERSION
    installed_assets = installed_manifest["assets"]["releases"][NEW_ASSET_VERSION]["arches"][arch]
    assert set(installed_assets) == {"vmlinuz", "initrd.img", "rootfs.erofs"}
    origin = json.loads((assets / "manifest-origin.json").read_text())
    assert origin["source"] == channel_manifest_url

    for name, blob in new_files.items():
        target = assets / arch / _hashed_asset_name(name, blob)
        assert target.exists(), f"{target} not downloaded. stdout={result.stdout}"
        assert target.read_bytes() == blob


def test_manifest_url_controls_asset_source(
    tmp_path: Path,
    http_fixture,
    installed_layout,
):
    base_url, serve_dir, requested_paths = http_fixture
    arch = _arch()

    files = {
        "vmlinuz": b"manifest-url-kernel-" + os.urandom(64),
        "initrd.img": b"manifest-url-initrd-" + os.urandom(64),
        "rootfs.erofs": b"manifest-url-rootfs-" + os.urandom(64),
    }

    channel_manifest_url = f"{base_url}/channels/nightly/manifest.json"
    channel_manifest = _make_manifest(arch, files, NEW_ASSET_VERSION)
    channel_manifest["asset_base"] = f"{base_url}/manifest-owned-assets/{{asset_version}}"
    channel_manifest_path = serve_dir / "channels" / "nightly" / "manifest.json"
    channel_manifest_path.parent.mkdir(parents=True)
    channel_manifest_path.write_text(json.dumps(channel_manifest), encoding="utf-8")

    release_dir = serve_dir / "manifest-owned-assets" / NEW_ASSET_VERSION
    release_dir.mkdir(parents=True)
    for name, blob in files.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    result = _run(
        {"CAPSEM_HOME": str(capsem_home)},
        "update",
        "--assets",
        "--manifest",
        channel_manifest_url,
    )

    assert result.returncode == 0, f"stdout={result.stdout}\nstderr={result.stderr}"
    assert f"Installed asset manifest from {channel_manifest_url}" in result.stdout
    assert "/channels/nightly/manifest.json" in requested_paths
    expected_blob_paths = {
        f"/manifest-owned-assets/{NEW_ASSET_VERSION}/{arch}-{name}" for name in files
    }
    assert expected_blob_paths.issubset(set(requested_paths))
    assert "/health.json" not in requested_paths
    assert set(requested_paths).isdisjoint(
        {f"/assets/releases/{NEW_ASSET_VERSION}/{arch}-{name}" for name in files}
    )

    assets = capsem_home / "assets"
    installed_manifest = json.loads((assets / "manifest.json").read_text())
    assert installed_manifest["asset_base"] == channel_manifest["asset_base"]
    assert installed_manifest["assets"]["current"] == NEW_ASSET_VERSION
    origin = json.loads((assets / "manifest-origin.json").read_text())
    assert origin["source"] == channel_manifest_url

    for name, blob in files.items():
        target = assets / arch / _hashed_asset_name(name, blob)
        assert target.exists(), f"{target} not downloaded. stdout={result.stdout}"
        assert target.read_bytes() == blob


def test_update_assets_failed_remote_refresh_keeps_previous_manifest_and_assets(
    tmp_path: Path,
    http_fixture,
    installed_layout,
):
    base_url, serve_dir, _requested_paths = http_fixture
    arch = _arch()

    old_files = {
        "vmlinuz": b"old-working-kernel",
        "initrd.img": b"old-working-initrd",
        "rootfs.erofs": b"old-working-rootfs",
    }
    new_declared_files = {
        "vmlinuz": b"new-declared-kernel",
        "initrd.img": b"new-declared-initrd",
        "rootfs.erofs": b"new-declared-rootfs",
    }

    channel_manifest_url = f"{base_url}/assets/stable/manifest.json"
    channel_manifest = _make_manifest(arch, new_declared_files, NEW_ASSET_VERSION)
    channel_manifest_path = serve_dir / "assets" / "stable" / "manifest.json"
    channel_manifest_path.parent.mkdir(parents=True)
    channel_manifest_path.write_text(json.dumps(channel_manifest), encoding="utf-8")

    release_dir = serve_dir / "assets" / "releases" / NEW_ASSET_VERSION
    release_dir.mkdir(parents=True)
    for name, blob in new_declared_files.items():
        served = b"corrupt-kernel-bytes" if name == "vmlinuz" else blob
        (release_dir / f"{arch}-{name}").write_bytes(served)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    arch_assets = assets / arch
    old_origin = {
        "schema": "capsem.manifest_origin.v1",
        "origin": "package",
        "source": channel_manifest_url,
        "packaged_at": "2026-06-16T00:00:00Z",
    }
    old_manifest = _write_installed_manifest_and_assets(
        assets,
        arch,
        old_files,
        asset_version=ASSET_VERSION,
        origin=old_origin,
    )

    result = _run({"CAPSEM_HOME": str(capsem_home)}, "update", "--assets")

    assert result.returncode != 0, f"expected corrupt channel update to fail: {result.stdout}"
    err = (result.stdout + result.stderr).lower()
    assert "hash mismatch" in err, err

    installed_manifest = json.loads((assets / "manifest.json").read_text())
    assert installed_manifest["assets"]["current"] == ASSET_VERSION
    assert installed_manifest == old_manifest
    assert json.loads((assets / "manifest-origin.json").read_text()) == old_origin

    for name, blob in old_files.items():
        target = arch_assets / _hashed_asset_name(name, blob)
        assert target.exists(), f"previous working asset was removed: {target}"
        assert target.read_bytes() == blob

    corrupt_target = arch_assets / _hashed_asset_name(
        "vmlinuz", new_declared_files["vmlinuz"]
    )
    assert not corrupt_target.exists(), "corrupt candidate must not replace old kernel"
    assert list(arch_assets.glob("*.tmp")) == []


def test_update_assets_rejects_incomplete_remote_manifest_without_clobbering_old_assets(
    tmp_path: Path,
    http_fixture,
    installed_layout,
):
    base_url, serve_dir, _requested_paths = http_fixture
    arch = _arch()

    old_files = {
        "vmlinuz": b"old-complete-kernel",
        "initrd.img": b"old-complete-initrd",
        "rootfs.erofs": b"old-complete-rootfs",
    }
    new_files = {
        "vmlinuz": b"new-incomplete-kernel",
        "initrd.img": b"new-incomplete-initrd",
        "rootfs.erofs": b"new-incomplete-rootfs",
    }

    channel_manifest_url = f"{base_url}/assets/stable/manifest.json"
    channel_manifest = _make_manifest(arch, new_files, NEW_ASSET_VERSION)
    del channel_manifest["assets"]["releases"][NEW_ASSET_VERSION]["arches"][arch]["rootfs.erofs"]
    channel_manifest_path = serve_dir / "assets" / "stable" / "manifest.json"
    channel_manifest_path.parent.mkdir(parents=True)
    channel_manifest_path.write_text(json.dumps(channel_manifest), encoding="utf-8")

    release_dir = serve_dir / "assets" / "releases" / NEW_ASSET_VERSION
    release_dir.mkdir(parents=True)
    for name, blob in new_files.items():
        if name != "rootfs.erofs":
            (release_dir / f"{arch}-{name}").write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    old_origin = {
        "schema": "capsem.manifest_origin.v1",
        "origin": "package",
        "source": channel_manifest_url,
        "packaged_at": "2026-06-16T00:00:00Z",
    }
    old_manifest = _write_installed_manifest_and_assets(
        assets,
        arch,
        old_files,
        asset_version=ASSET_VERSION,
        origin=old_origin,
    )

    result = _run({"CAPSEM_HOME": str(capsem_home)}, "update", "--assets")

    assert result.returncode != 0, "manifest without rootfs must not hydrate successfully"
    err = (result.stdout + result.stderr).lower()
    assert "rootfs not found" in err, err
    assert "restored previous installed manifest" in err, err
    assert json.loads((assets / "manifest.json").read_text()) == old_manifest
    assert json.loads((assets / "manifest-origin.json").read_text()) == old_origin
    for name, blob in old_files.items():
        target = assets / arch / _hashed_asset_name(name, blob)
        assert target.exists(), f"previous working asset was removed: {target}"
        assert target.read_bytes() == blob


def test_update_assets_interrupted_remote_blob_keeps_previous_manifest_and_cleans_tmp(
    tmp_path: Path,
    installed_layout,
):
    arch = _arch()
    old_files = {
        "vmlinuz": b"old-stream-kernel",
        "initrd.img": b"old-stream-initrd",
        "rootfs.erofs": b"old-stream-rootfs",
    }
    new_files = {
        "vmlinuz": b"new-stream-kernel" * 1024,
        "initrd.img": b"new-stream-initrd" * 1024,
        "rootfs.erofs": b"new-stream-rootfs" * 1024,
    }
    channel_manifest = _make_manifest(arch, new_files, NEW_ASSET_VERSION)
    interrupted_path = f"/assets/releases/{NEW_ASSET_VERSION}/{arch}-rootfs.erofs"
    blobs = {
        f"/assets/releases/{NEW_ASSET_VERSION}/{arch}-{name}": blob
        for name, blob in new_files.items()
    }

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"

    with _serve_interrupted_asset_channel(channel_manifest, blobs, interrupted_path) as base_url:
        channel_manifest_url = f"{base_url}/assets/stable/manifest.json"
        old_origin = {
            "schema": "capsem.manifest_origin.v1",
            "origin": "package",
            "source": channel_manifest_url,
            "packaged_at": "2026-06-16T00:00:00Z",
        }
        old_manifest = _write_installed_manifest_and_assets(
            assets,
            arch,
            old_files,
            asset_version=ASSET_VERSION,
            origin=old_origin,
        )

        result = _run({"CAPSEM_HOME": str(capsem_home)}, "update", "--assets")

    assert result.returncode != 0, "interrupted asset stream must fail the update"
    err = (result.stdout + result.stderr).lower()
    assert "asset refresh failed" in err, err
    assert "restored previous installed manifest" in err, err
    assert json.loads((assets / "manifest.json").read_text()) == old_manifest
    assert json.loads((assets / "manifest-origin.json").read_text()) == old_origin

    arch_assets = assets / arch
    for name, blob in old_files.items():
        target = arch_assets / _hashed_asset_name(name, blob)
        assert target.exists(), f"previous working asset was removed: {target}"
        assert target.read_bytes() == blob
    assert list(arch_assets.glob("*.tmp")) == []


def test_update_assets_missing_remote_channel_manifest_keeps_previous_assets(
    tmp_path: Path,
    http_fixture,
    installed_layout,
):
    base_url, _serve_dir, requested_paths = http_fixture
    arch = _arch()
    old_files = {
        "vmlinuz": b"old-offline-kernel",
        "initrd.img": b"old-offline-initrd",
        "rootfs.erofs": b"old-offline-rootfs",
    }

    channel_manifest_url = f"{base_url}/assets/stable/manifest.json"
    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    old_origin = {
        "schema": "capsem.manifest_origin.v1",
        "origin": "package",
        "source": channel_manifest_url,
        "packaged_at": "2026-06-16T00:00:00Z",
    }
    old_manifest = _write_installed_manifest_and_assets(
        assets,
        arch,
        old_files,
        asset_version=ASSET_VERSION,
        origin=old_origin,
    )

    result = _run({"CAPSEM_HOME": str(capsem_home)}, "update", "--assets")

    assert result.returncode != 0, "missing channel manifest must fail the refresh"
    err = (result.stdout + result.stderr).lower()
    assert "manifest.json" in err, err
    assert "404" in err or "not found" in err, err
    assert "/assets/stable/manifest.json" in requested_paths
    assert json.loads((assets / "manifest.json").read_text()) == old_manifest
    assert json.loads((assets / "manifest-origin.json").read_text()) == old_origin
    for name, blob in old_files.items():
        target = assets / arch / _hashed_asset_name(name, blob)
        assert target.exists(), f"previous working asset was removed: {target}"
        assert target.read_bytes() == blob


def test_update_assets_deprecated_remote_release_keeps_previous_assets(
    tmp_path: Path,
    http_fixture,
    installed_layout,
):
    base_url, serve_dir, requested_paths = http_fixture
    arch = _arch()
    old_files = {
        "vmlinuz": b"old-deprecated-kernel",
        "initrd.img": b"old-deprecated-initrd",
        "rootfs.erofs": b"old-deprecated-rootfs",
    }
    deprecated_files = {
        "vmlinuz": b"deprecated-kernel",
        "initrd.img": b"deprecated-initrd",
        "rootfs.erofs": b"deprecated-rootfs",
    }

    channel_manifest_url = f"{base_url}/assets/stable/manifest.json"
    channel_manifest = _make_manifest(arch, deprecated_files, NEW_ASSET_VERSION)
    release = channel_manifest["assets"]["releases"][NEW_ASSET_VERSION]
    release["deprecated"] = True
    release["deprecated_date"] = "2030-01-03"
    channel_manifest_path = serve_dir / "assets" / "stable" / "manifest.json"
    channel_manifest_path.parent.mkdir(parents=True)
    channel_manifest_path.write_text(json.dumps(channel_manifest), encoding="utf-8")

    release_dir = serve_dir / "assets" / "releases" / NEW_ASSET_VERSION
    release_dir.mkdir(parents=True)
    for name, blob in deprecated_files.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    old_origin = {
        "schema": "capsem.manifest_origin.v1",
        "origin": "package",
        "source": channel_manifest_url,
        "packaged_at": "2026-06-16T00:00:00Z",
    }
    old_manifest = _write_installed_manifest_and_assets(
        assets,
        arch,
        old_files,
        asset_version=ASSET_VERSION,
        origin=old_origin,
    )

    result = _run({"CAPSEM_HOME": str(capsem_home)}, "update", "--assets")

    assert result.returncode != 0, "deprecated-only channel must not replace installed assets"
    err = result.stdout + result.stderr
    assert "no compatible asset release" in err, err
    assert "restored previous installed manifest" in err, err
    assert "/assets/stable/manifest.json" in requested_paths
    deprecated_blob_paths = {
        f"/assets/releases/{NEW_ASSET_VERSION}/{arch}-{name}"
        for name in deprecated_files
    }
    assert set(requested_paths).isdisjoint(deprecated_blob_paths), requested_paths
    assert json.loads((assets / "manifest.json").read_text()) == old_manifest
    assert json.loads((assets / "manifest-origin.json").read_text()) == old_origin
    for name, blob in old_files.items():
        target = assets / arch / _hashed_asset_name(name, blob)
        assert target.exists(), f"previous working asset was removed: {target}"
        assert target.read_bytes() == blob


def test_update_assets_rejects_bare_asset_base_path(tmp_path: Path, installed_layout):
    arch = _arch()
    files = {
        "vmlinuz": b"k",
        "initrd.img": b"i",
        "rootfs.erofs": b"r",
    }
    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)
    (assets / "manifest.json").write_text(json.dumps(_make_manifest(arch, files)))

    bare_release_path = tmp_path / "release"
    bare_release_path.mkdir()
    env = {
        "CAPSEM_HOME": str(capsem_home),
        "CAPSEM_ASSET_BASE_URL": str(bare_release_path),
    }

    r = _run(env, "update", "--assets")

    assert r.returncode != 0
    err = r.stdout + r.stderr
    assert "asset base URL must be a URL" in err, err


def test_update_assets_rejects_manifest_assets_requiring_newer_binary(
    tmp_path: Path,
    installed_layout,
):
    arch = _arch()
    files = {
        "vmlinuz": b"k",
        "initrd.img": b"i",
        "rootfs.erofs": b"r",
    }
    manifest = _make_manifest(arch, files)
    manifest["assets"]["releases"][ASSET_VERSION]["min_binary"] = "9999.0.0"

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)
    (assets / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

    result = _run({"CAPSEM_HOME": str(capsem_home)}, "update", "--assets")

    assert result.returncode != 0
    err = result.stdout + result.stderr
    assert "no compatible asset release" in err, err


def test_update_assets_idempotent_when_hashes_match(tmp_path: Path, http_fixture, installed_layout):
    base_url, serve_dir, _requested_paths = http_fixture
    arch = _arch()

    files = {
        "vmlinuz": b"kern",
        "initrd.img": b"initrd",
        "rootfs.erofs": b"rootfs",
    }
    release_dir = serve_dir / ASSET_VERSION
    release_dir.mkdir()
    for name, blob in files.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)
    (assets / "manifest.json").write_text(json.dumps(_make_manifest(arch, files)))

    env = {"CAPSEM_HOME": str(capsem_home), "CAPSEM_ASSET_BASE_URL": base_url}
    r1 = _run(env, "update", "--assets")
    assert r1.returncode == 0

    # Second run: remove the server so the command must succeed purely from
    # the hash check short-circuit.
    for f in release_dir.iterdir():
        f.unlink()
    r2 = _run(env, "update", "--assets")
    assert r2.returncode == 0, f"idempotent rerun failed: {r2.stderr}"
    assert "up to date" in r2.stdout.lower() or "already" in r2.stdout.lower()


def test_update_assets_hash_mismatch_fails(tmp_path: Path, http_fixture, installed_layout):
    base_url, serve_dir, _requested_paths = http_fixture
    arch = _arch()

    declared = {"vmlinuz": b"the-right-bytes"}
    served_blob = b"the-WRONG-bytes"

    release_dir = serve_dir / ASSET_VERSION
    release_dir.mkdir()
    (release_dir / f"{arch}-vmlinuz").write_bytes(served_blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)

    manifest = _make_manifest(arch, declared)
    # The manifest must also declare initrd/rootfs or resolve() fails before
    # getting to vmlinuz. Serve matching bytes for those so only vmlinuz fails.
    extras = {"initrd.img": b"i", "rootfs.erofs": b"r"}
    for name, blob in extras.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)
    manifest["assets"]["releases"][ASSET_VERSION]["arches"][arch].update(
        {name: {"hash": _blake3(blob), "size": len(blob)} for name, blob in extras.items()}
    )
    (assets / "manifest.json").write_text(json.dumps(manifest))

    env = {"CAPSEM_HOME": str(capsem_home), "CAPSEM_ASSET_BASE_URL": base_url}
    r = _run(env, "update", "--assets")
    assert r.returncode != 0, f"expected failure, stdout={r.stdout}"
    assert "hash mismatch" in (r.stdout + r.stderr).lower()
    # No .tmp file should be left behind.
    leftovers = list((assets / arch).glob("*.tmp")) if (assets / arch).exists() else []
    assert leftovers == [], f"stale tmp files: {leftovers}"


def test_update_assets_404_fails(tmp_path: Path, http_fixture, installed_layout):
    base_url, serve_dir, _requested_paths = http_fixture
    arch = _arch()

    files = {
        "vmlinuz": b"k",
        "initrd.img": b"i",
        "rootfs.erofs": b"r",
    }
    release_dir = serve_dir / ASSET_VERSION
    release_dir.mkdir()
    # Serve only two of three to force a 404.
    (release_dir / f"{arch}-vmlinuz").write_bytes(files["vmlinuz"])
    (release_dir / f"{arch}-initrd.img").write_bytes(files["initrd.img"])

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)
    (assets / "manifest.json").write_text(json.dumps(_make_manifest(arch, files)))

    env = {"CAPSEM_HOME": str(capsem_home), "CAPSEM_ASSET_BASE_URL": base_url}
    r = _run(env, "update", "--assets")
    assert r.returncode != 0
    err = (r.stdout + r.stderr).lower()
    assert "404" in err or "not found" in err, err
