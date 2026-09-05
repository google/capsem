"""Unit tests for build_system/scripts/build/clean_stale.py."""

from __future__ import annotations

import json
import os
import shutil
import socket
import tempfile
import time
from pathlib import Path

import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.image.tools.build import clean_stale

REPO_ROOT = Path(__file__).resolve().parents[3]
def _make_orphan_socket(path: Path) -> None:
    """Create a UDS file with no listener (bind, then close)."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        s.bind(str(path))
    finally:
        s.close()
    # bind() leaves the file on disk; closing it without listen() means
    # connect() will hit ECONNREFUSED -- exactly the orphan condition.


@pytest.fixture
def live_listener():
    """Yield a (path, listener_socket) pair; caller provides path."""
    holders: list[socket.socket] = []

    def _make(path: Path):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.bind(str(path))
        s.listen(1)
        holders.append(s)
        return s

    yield _make
    for s in holders:
        s.close()


@pytest.fixture
def short_sock_dir():
    """AF_UNIX paths on macOS are capped at 104 chars. pytest's tmp_path lives
    under /private/var/folders/... which already exceeds that. Give tests a
    short /tmp-rooted dir just for socket files."""
    d = Path(tempfile.mkdtemp(prefix="capsem-clean-stale-", dir="/tmp"))
    try:
        yield d
    finally:
        shutil.rmtree(d, ignore_errors=True)


def test_orphan_socket_removed(short_sock_dir: Path):
    sock = short_sock_dir / "dead.sock"
    _make_orphan_socket(sock)
    assert sock.exists()

    result = clean_stale.clean_orphan_sockets(short_sock_dir, dry_run=False, verbose=False)

    assert result.removed == 1
    assert not sock.exists()


def test_listening_socket_kept(short_sock_dir: Path, live_listener):
    sock = short_sock_dir / "live.sock"
    live_listener(sock)
    assert sock.exists()

    result = clean_stale.clean_orphan_sockets(short_sock_dir, dry_run=False, verbose=False)

    assert result.removed == 0
    assert sock.exists()


def test_ready_companion_removed(short_sock_dir: Path):
    sock = short_sock_dir / "dead.sock"
    ready = short_sock_dir / "dead.ready"
    _make_orphan_socket(sock)
    ready.write_text("")

    clean_stale.clean_orphan_sockets(short_sock_dir, dry_run=False, verbose=False)

    assert not sock.exists()
    assert not ready.exists()


def test_ready_companion_of_live_sock_kept(short_sock_dir: Path, live_listener):
    sock = short_sock_dir / "live.sock"
    ready = short_sock_dir / "live.ready"
    live_listener(sock)
    ready.write_text("")

    clean_stale.clean_orphan_sockets(short_sock_dir, dry_run=False, verbose=False)

    assert sock.exists()
    assert ready.exists()


def test_mixed_socket_batch(short_sock_dir: Path, live_listener):
    live = short_sock_dir / "live.sock"
    dead1 = short_sock_dir / "dead1.sock"
    dead2 = short_sock_dir / "dead2.sock"
    live_listener(live)
    _make_orphan_socket(dead1)
    _make_orphan_socket(dead2)

    result = clean_stale.clean_orphan_sockets(short_sock_dir, dry_run=False, verbose=False)

    assert result.removed == 2
    assert live.exists()
    assert not dead1.exists()
    assert not dead2.exists()


def test_perf_many_orphan_sockets(short_sock_dir: Path):
    """Regression guard against reintroducing per-socket lsof (~200ms each)."""
    count = 2000
    for i in range(count):
        _make_orphan_socket(short_sock_dir / f"s{i}.sock")

    start = time.monotonic()
    result = clean_stale.clean_orphan_sockets(short_sock_dir, dry_run=False, verbose=False)
    elapsed = time.monotonic() - start

    assert result.removed == count
    # Generous cap; should typically land well under 1s.
    assert elapsed < 2.0, f"stage took {elapsed:.2f}s for {count} sockets"


def test_stale_rootfs_dir_removed(tmp_path: Path):
    debug = tmp_path / "cache" / "target" / "cargo" / "debug"
    debug.mkdir(parents=True)
    rootfs = debug / "rootfs.abc123"
    rootfs.mkdir()
    (rootfs / "marker").write_text("x")

    release = tmp_path / "cache" / "target" / "cargo" / "release"
    release.mkdir(parents=True)
    rootfs_rel = release / "rootfs.xyz"
    rootfs_rel.mkdir()

    llvm_debug = tmp_path / "cache" / "target" / "cargo" / "coverage" / "debug"
    llvm_debug.mkdir(parents=True)
    rootfs_llvm = llvm_debug / "rootfs.q"
    rootfs_llvm.mkdir()

    up_dir = tmp_path / "cache" / "target" / "cargo" / "debug" / "something" / "_up_"
    up_dir.mkdir(parents=True)
    (up_dir / "marker").write_text("y")

    result = clean_stale.clean_rootfs_scratch(tmp_path, dry_run=False, verbose=False)

    assert result.removed == 4
    assert not rootfs.exists()
    assert not rootfs_rel.exists()
    assert not rootfs_llvm.exists()
    assert not up_dir.exists()


def test_live_rootfs_artifact_untouched(tmp_path: Path):
    """A file named rootfs.xyz that's a real build product (file) must be kept.

    Our matcher requires a directory named rootfs.*; a plain file should not
    match. Also verify unrelated binaries in cache/target/cargo/debug/ are untouched.
    """
    debug = tmp_path / "cache" / "target" / "cargo" / "debug"
    debug.mkdir(parents=True)

    # Real build artifact (not a dir, not under a matching parent pattern).
    binary = debug / "capsem"
    binary.write_text("fake binary")

    # File (not dir) that happens to match the rootfs.* name.
    weird_file = debug / "rootfs.meta"
    weird_file.write_text("not a dir")

    # Unrelated subdir that is not named rootfs.*.
    other = debug / "deps"
    other.mkdir()
    (other / "libcapsem.rlib").write_text("x")

    result = clean_stale.clean_rootfs_scratch(tmp_path, dry_run=False, verbose=False)

    assert result.removed == 0
    assert binary.exists()
    assert weird_file.exists()
    assert other.exists()


def test_target_transient_cleanup_removes_only_old_reproducible_outputs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """Old proof/debug staging is disposable; canonical asset caches are not."""
    monkeypatch.setattr(clean_stale, "TARGET_TRANSIENT_MAX_AGE_S", 60)
    target = tmp_path / "cache" / "target"
    target.mkdir(parents=True)
    old_time = time.time() - 3600

    stale_names = (
        "asset-release",
        "generated-settings-linux.abc123",
        "local-release-glowup-debug",
        "agy-proof-arm64",
        "ironbank-assets-sequential",
        "s09-043-release-dist",
    )
    for name in stale_names:
        path = target / name
        path.mkdir()
        (path / "payload").write_bytes(b"stale")
        os.utime(path, (old_time, old_time))

    protected = target / "ironbank-assets"
    protected.mkdir()
    (protected / "rootfs.erofs").write_bytes(b"expensive current cache")
    os.utime(protected, (old_time, old_time))

    fresh = target / "local-release-glowup"
    fresh.mkdir()
    (fresh / "report.json").write_text("{}")

    result = clean_stale.clean_target_transients(tmp_path, dry_run=False, verbose=False)

    assert result.removed == len(stale_names)
    assert all(not (target / name).exists() for name in stale_names)
    assert protected.exists(), "canonical Ironbank assets accelerate the next gate"
    assert fresh.exists(), "a possibly active staging tree must not be removed"


def test_target_tmp_cleanup_removes_old_scratch_but_preserves_fresh_work(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(clean_stale, "TARGET_TRANSIENT_MAX_AGE_S", 60)
    scratch = tmp_path / "cache" / "target" / "tmp"
    scratch.mkdir(parents=True)
    old = scratch / "obom-debug-rootfs"
    old.mkdir()
    (old / "rootfs.tar").write_bytes(b"stale")
    old_time = time.time() - 3600
    os.utime(old, (old_time, old_time))
    fresh = scratch / "capsem-build-rootfs-active"
    fresh.mkdir()

    result = clean_stale.clean_target_transients(tmp_path, dry_run=False, verbose=False)

    assert result.removed == 1
    assert not old.exists()
    assert fresh.exists()


def test_dry_run_removes_nothing(tmp_path: Path, short_sock_dir: Path):
    debug = tmp_path / "cache" / "target" / "cargo" / "debug"
    debug.mkdir(parents=True)
    rootfs = debug / "rootfs.abc"
    rootfs.mkdir()

    sock = short_sock_dir / "dead.sock"
    _make_orphan_socket(sock)

    # All stages with dry_run=True must keep files intact but still report counts.
    ra = clean_stale.clean_rootfs_scratch(tmp_path, dry_run=True, verbose=False)
    rb = clean_stale.clean_orphan_sockets(short_sock_dir, dry_run=True, verbose=False)

    assert ra.removed == 1 and rootfs.exists()
    assert rb.removed == 1 and sock.exists()


def test_sockets_dir_missing(tmp_path: Path):
    """Missing sockets dir is not an error; returns zero removed."""
    result = clean_stale.clean_orphan_sockets(
        tmp_path / "does-not-exist", dry_run=False, verbose=False
    )
    assert result.removed == 0


def test_target_missing(tmp_path: Path):
    """Missing cache/target/ dir is not an error."""
    ra = clean_stale.clean_rootfs_scratch(tmp_path, dry_run=False, verbose=False)
    assert ra.removed == 0


def test_main_persists_cleanup_ledger(tmp_path: Path):
    report = tmp_path / "cleanup.jsonl"
    rc = clean_stale.main(
        [
            "--root",
            str(tmp_path),
            "--sockets-dir",
            str(tmp_path / "sockets"),
            "--report",
            str(report),
        ]
    )

    assert rc == 0
    payload = json.loads(report.read_text().splitlines()[-1])
    assert payload["schema"] == "capsem.host_cleanup.v1"
    assert payload["target"]["before_bytes"] >= payload["target"]["after_bytes"]
    assert all(stage["name"] != "cargo" for stage in payload["stages"])


def test_main_places_default_ledger_in_policy_owned_state(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    config = tmp_path / "config"
    config.mkdir()
    shutil.copy2(REPO_ROOT / "config/cache.toml", config / "cache.toml")
    monkeypatch.delenv(load_policy(tmp_path).authority_environment, raising=False)

    rc = clean_stale.main(
        [
            "--root",
            str(tmp_path),
            "--sockets-dir",
            str(tmp_path / "sockets"),
        ]
    )

    assert rc == 0
    ledger = tmp_path / "cache/state/host-cleanup.jsonl"
    assert json.loads(ledger.read_text().splitlines()[-1])["schema"] == (
        "capsem.host_cleanup.v1"
    )
