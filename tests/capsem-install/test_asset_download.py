"""End-to-end test for `capsem update --assets` against a local HTTP fixture.

Validates the asset download path wired through
`capsem_core::asset_manager::download_missing_assets`:

  - happy path: files land at `<CAPSEM_HOME>/assets/<arch>/<hash-filename>`
    with matching blake3 and 0o444 perms
  - hash mismatch: server serves wrong bytes -> command fails, no file left
  - 404: release URL missing a file -> command fails with URL in error

The server is a threaded `http.server` bound to 127.0.0.1:0. The test writes a
minimal v2 manifest whose hashes match the fixture bytes, then runs the CLI
against `CAPSEM_HOME` + `CAPSEM_RELEASE_URL` pointed at the server.
"""

from __future__ import annotations

import json
import os
import platform
import subprocess
import threading
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CAPSEM_BIN = REPO_ROOT / "target" / "debug" / "capsem"


def _arch() -> str:
    m = platform.machine().lower()
    if m in ("arm64", "aarch64"):
        return "arm64"
    return "x86_64"


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


def _make_manifest(arch: str, files: dict[str, bytes]) -> dict:
    """Build a minimal v2 manifest for the given arch + byte blobs."""
    return {
        "format": 2,
        "assets": {
            "current": "2030.0101.1",
            "releases": {
                "2030.0101.1": {
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
                    "min_assets": "2030.0101.1",
                }
            },
        },
    }


@pytest.fixture
def http_fixture(tmp_path: Path):
    """Spin an http.server in the background; yield (base_url, serve_dir)."""
    serve_dir = tmp_path / "release"
    serve_dir.mkdir()

    serve_str = str(serve_dir)

    class Handler(SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            kwargs["directory"] = serve_str
            super().__init__(*args, **kwargs)

        def log_message(self, format, *args):
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    host, port = server.server_address
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield (f"http://{host}:{port}", serve_dir)
    finally:
        server.shutdown()
        server.server_close()


def _run(env: dict, *args: str) -> subprocess.CompletedProcess:
    assert CAPSEM_BIN.exists(), (
        f"capsem binary not built at {CAPSEM_BIN}; "
        "run `cargo build -p capsem` first"
    )
    return subprocess.run(
        [str(CAPSEM_BIN), *args],
        capture_output=True,
        text=True,
        timeout=30,
        env={**os.environ, **env},
    )


def test_update_assets_downloads_missing(tmp_path: Path, http_fixture):
    base_url, serve_dir = http_fixture
    arch = _arch()

    # Fixture bytes: small so hashing is cheap but non-empty.
    files = {
        "vmlinuz": b"test-kernel-bytes-" + os.urandom(64),
        "initrd.img": b"test-initrd-bytes-" + os.urandom(64),
        "rootfs.squashfs": b"test-rootfs-bytes-" + os.urandom(64),
    }

    # Asset version directory matches release_url(asset_version) pattern.
    release_dir = serve_dir / "v2030.0101.1"
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
        "CAPSEM_RELEASE_URL": base_url,
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


def test_update_assets_idempotent_when_hashes_match(tmp_path: Path, http_fixture):
    base_url, serve_dir = http_fixture
    arch = _arch()

    files = {
        "vmlinuz": b"kern",
        "initrd.img": b"initrd",
        "rootfs.squashfs": b"rootfs",
    }
    release_dir = serve_dir / "v2030.0101.1"
    release_dir.mkdir()
    for name, blob in files.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)
    (assets / "manifest.json").write_text(json.dumps(_make_manifest(arch, files)))

    env = {"CAPSEM_HOME": str(capsem_home), "CAPSEM_RELEASE_URL": base_url}
    r1 = _run(env, "update", "--assets")
    assert r1.returncode == 0

    # Second run: remove the server so the command must succeed purely from
    # the hash check short-circuit.
    for f in release_dir.iterdir():
        f.unlink()
    r2 = _run(env, "update", "--assets")
    assert r2.returncode == 0, f"idempotent rerun failed: {r2.stderr}"
    assert "up to date" in r2.stdout.lower() or "already" in r2.stdout.lower()


def test_update_assets_hash_mismatch_fails(tmp_path: Path, http_fixture):
    base_url, serve_dir = http_fixture
    arch = _arch()

    declared = {"vmlinuz": b"the-right-bytes"}
    served_blob = b"the-WRONG-bytes"

    release_dir = serve_dir / "v2030.0101.1"
    release_dir.mkdir()
    (release_dir / f"{arch}-vmlinuz").write_bytes(served_blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)

    manifest = _make_manifest(arch, declared)
    # The manifest must also declare initrd/rootfs or resolve() fails before
    # getting to vmlinuz. Serve matching bytes for those so only vmlinuz fails.
    extras = {"initrd.img": b"i", "rootfs.squashfs": b"r"}
    for name, blob in extras.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)
    manifest["assets"]["releases"]["2030.0101.1"]["arches"][arch].update(
        {name: {"hash": _blake3(blob), "size": len(blob)} for name, blob in extras.items()}
    )
    (assets / "manifest.json").write_text(json.dumps(manifest))

    env = {"CAPSEM_HOME": str(capsem_home), "CAPSEM_RELEASE_URL": base_url}
    r = _run(env, "update", "--assets")
    assert r.returncode != 0, f"expected failure, stdout={r.stdout}"
    assert "hash mismatch" in (r.stdout + r.stderr).lower()
    # No .tmp file should be left behind.
    leftovers = list((assets / arch).glob("*.tmp")) if (assets / arch).exists() else []
    assert leftovers == [], f"stale tmp files: {leftovers}"


def test_update_assets_404_fails(tmp_path: Path, http_fixture):
    base_url, serve_dir = http_fixture
    arch = _arch()

    files = {
        "vmlinuz": b"k",
        "initrd.img": b"i",
        "rootfs.squashfs": b"r",
    }
    release_dir = serve_dir / "v2030.0101.1"
    release_dir.mkdir()
    # Serve only two of three to force a 404.
    (release_dir / f"{arch}-vmlinuz").write_bytes(files["vmlinuz"])
    (release_dir / f"{arch}-initrd.img").write_bytes(files["initrd.img"])

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)
    (assets / "manifest.json").write_text(json.dumps(_make_manifest(arch, files)))

    env = {"CAPSEM_HOME": str(capsem_home), "CAPSEM_RELEASE_URL": base_url}
    r = _run(env, "update", "--assets")
    assert r.returncode != 0
    err = (r.stdout + r.stderr).lower()
    assert "404" in err or "not found" in err, err
