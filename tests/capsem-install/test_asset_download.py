"""End-to-end test for `capsem update --assets` against a local HTTP fixture.

Validates the asset download path wired through
the service-owned Profile V2 asset reconciler:

  - happy path: files land at `<CAPSEM_HOME>/assets/<arch>/<hash-filename>`
    with matching blake3 and 0o444 perms
  - hash mismatch: server serves wrong bytes -> command fails, no file left
  - 404: profile URL missing a file -> command fails with URL in error

The server is a threaded `http.server` bound to 127.0.0.1:0. The test writes a
minimal Profile V2 TOML whose asset declarations point at the fixture, then
runs the CLI against an isolated `CAPSEM_HOME`.
"""

from __future__ import annotations

import os
import platform
import subprocess
import tempfile
import threading
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
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


def _binary_version() -> str:
    """Query the installed binary's compiled-in version.

    asset_manager constructs URLs from CARGO_PKG_VERSION (the running binary),
    not the manifest's binaries.current. The fixture release dir has to match
    that or every download 404s. Cached -- shells out once per test session.
    """
    out = subprocess.check_output([str(CAPSEM_BIN), "--version"], text=True)
    # `capsem 1.0.1777065213` -> `1.0.1777065213`
    return out.strip().split()[-1]


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


def _write_profile_backed_service(capsem_home: Path, arch: str, base_url: str, files: dict[str, bytes]) -> None:
    """Install a trusted base profile whose VM assets point at the HTTP fixture."""
    profiles = capsem_home / "profiles" / "base"
    profiles.mkdir(parents=True, exist_ok=True)
    assets = capsem_home / "assets"
    assets.mkdir(parents=True, exist_ok=True)
    release_url = f"{base_url}/v{_binary_version()}"

    def asset_section(field: str, logical: str, content_type: str) -> str:
        blob = files[logical]
        return f"""
[vm.assets.{arch}.{field}]
url = "{release_url}/{arch}-{logical}"
hash = "blake3:{_blake3(blob)}"
signature_url = "{release_url}/{arch}-{logical}.minisig"
size = {len(blob)}
content_type = "{content_type}"
"""

    profile = f"""
schema = "capsem.profile.v2"
version = 1
id = "everyday-work"
revision = "2030.0101.1"
name = "Everyday Work"
description = "Install test profile."
best_for = "Install test profile."
profile_type = "everyday-work"
ui = "everyday"

[compatibility]
min_binary = "1.0.0"
guest_abi = "capsem-guest-v2"

[vm]
memory_mib = 1024
cpus = 1
disk_mib = 1024
network = "proxied"
""".lstrip()
    profile += asset_section("kernel", "vmlinuz", "application/octet-stream")
    profile += asset_section("initrd", "initrd.img", "application/octet-stream")
    profile += asset_section("rootfs", "rootfs.squashfs", "application/vnd.squashfs")
    (profiles / "everyday-work.profile.toml").write_text(profile, encoding="utf-8")

    service = f"""
version = 1

[profiles]
base_dirs = ["{profiles}"]
corp_dirs = []
user_dirs = []
default_profile = "everyday-work"
allow_user_profiles = true
allow_user_fork = true
allow_user_delete = true

[assets]
assets_dir = "{assets}"
image_roots = []
""".lstrip()
    (capsem_home / "service.toml").write_text(service, encoding="utf-8")


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


def _env_for_home(capsem_home: Path) -> dict[str, str]:
    # Keep the service socket path short enough for macOS sockaddr_un while
    # leaving durable profile/assets state in the pytest temp home.
    run_dir = Path(tempfile.mkdtemp(prefix="capsem-assets-", dir="/tmp"))
    return {
        "CAPSEM_HOME": str(capsem_home),
        "CAPSEM_RUN_DIR": str(run_dir),
    }


def test_update_assets_downloads_missing(tmp_path: Path, http_fixture, installed_layout):
    base_url, serve_dir = http_fixture
    arch = _arch()

    # Fixture bytes: small so hashing is cheap but non-empty.
    files = {
        "vmlinuz": b"test-kernel-bytes-" + os.urandom(64),
        "initrd.img": b"test-initrd-bytes-" + os.urandom(64),
        "rootfs.squashfs": b"test-rootfs-bytes-" + os.urandom(64),
    }

    release_dir = serve_dir / f"v{_binary_version()}"
    release_dir.mkdir()
    for name, blob in files.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    _write_profile_backed_service(capsem_home, arch, base_url, files)

    env = _env_for_home(capsem_home)
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


def test_update_assets_idempotent_when_hashes_match(tmp_path: Path, http_fixture, installed_layout):
    base_url, serve_dir = http_fixture
    arch = _arch()

    files = {
        "vmlinuz": b"kern",
        "initrd.img": b"initrd",
        "rootfs.squashfs": b"rootfs",
    }
    release_dir = serve_dir / f"v{_binary_version()}"
    release_dir.mkdir()
    for name, blob in files.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    _write_profile_backed_service(capsem_home, arch, base_url, files)

    env = _env_for_home(capsem_home)
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
    base_url, serve_dir = http_fixture
    arch = _arch()

    declared = {"vmlinuz": b"the-right-bytes"}
    served_blob = b"the-WRONG-bytes"

    release_dir = serve_dir / f"v{_binary_version()}"
    release_dir.mkdir()
    (release_dir / f"{arch}-vmlinuz").write_bytes(served_blob)

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    assets.mkdir(parents=True)

    # The profile must also declare initrd/rootfs or service startup fails before
    # getting to vmlinuz. Serve matching bytes for those so only vmlinuz fails.
    extras = {"initrd.img": b"i", "rootfs.squashfs": b"r"}
    files = {**declared, **extras}
    for name, blob in extras.items():
        (release_dir / f"{arch}-{name}").write_bytes(blob)
    _write_profile_backed_service(capsem_home, arch, base_url, files)

    env = _env_for_home(capsem_home)
    r = _run(env, "update", "--assets")
    assert r.returncode != 0, f"expected failure, stdout={r.stdout}"
    assert "hash mismatch" in (r.stdout + r.stderr).lower()
    # No .tmp file should be left behind.
    leftovers = list((assets / arch).glob("*.tmp")) if (assets / arch).exists() else []
    assert leftovers == [], f"stale tmp files: {leftovers}"


def test_update_assets_404_fails(tmp_path: Path, http_fixture, installed_layout):
    base_url, serve_dir = http_fixture
    arch = _arch()

    files = {
        "vmlinuz": b"k",
        "initrd.img": b"i",
        "rootfs.squashfs": b"r",
    }
    release_dir = serve_dir / f"v{_binary_version()}"
    release_dir.mkdir()
    # Serve only two of three to force a 404.
    (release_dir / f"{arch}-vmlinuz").write_bytes(files["vmlinuz"])
    (release_dir / f"{arch}-initrd.img").write_bytes(files["initrd.img"])

    capsem_home = tmp_path / ".capsem"
    assets = capsem_home / "assets"
    _write_profile_backed_service(capsem_home, arch, base_url, files)

    env = _env_for_home(capsem_home)
    r = _run(env, "update", "--assets")
    assert r.returncode != 0
    err = (r.stdout + r.stderr).lower()
    assert "404" in err or "not found" in err, err
