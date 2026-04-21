"""Unit tests for scripts/clean_stale.py."""

from __future__ import annotations

import importlib.util
import os
import shutil
import socket
import sys
import tempfile
import time
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "clean_stale.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("clean_stale", SCRIPT_PATH)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules["clean_stale"] = module  # dataclass needs sys.modules lookup
    spec.loader.exec_module(module)
    return module


clean_stale = _load_module()


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
    d = Path(tempfile.mkdtemp(prefix="cs-", dir="/tmp"))
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
    debug = tmp_path / "target" / "debug"
    debug.mkdir(parents=True)
    rootfs = debug / "rootfs.abc123"
    rootfs.mkdir()
    (rootfs / "marker").write_text("x")

    release = tmp_path / "target" / "release"
    release.mkdir(parents=True)
    rootfs_rel = release / "rootfs.xyz"
    rootfs_rel.mkdir()

    llvm_debug = tmp_path / "target" / "llvm-cov-target" / "debug"
    llvm_debug.mkdir(parents=True)
    rootfs_llvm = llvm_debug / "rootfs.q"
    rootfs_llvm.mkdir()

    up_dir = tmp_path / "target" / "debug" / "something" / "_up_"
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
    match. Also verify unrelated binaries in target/debug/ are untouched.
    """
    debug = tmp_path / "target" / "debug"
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


def test_old_tmp_fixture_removed(tmp_path: Path):
    tmp = tmp_path / "T"
    tmp.mkdir()
    stale = tmp / "capsem-test-abc"
    stale.mkdir()
    old_time = time.time() - 2 * 3600  # 2 hours ago
    os.utime(stale, (old_time, old_time))

    result = clean_stale.clean_tmp_fixtures(tmp, dry_run=False, verbose=False)

    assert result.removed == 1
    assert not stale.exists()


def test_recent_tmp_fixture_kept(tmp_path: Path):
    tmp = tmp_path / "T"
    tmp.mkdir()
    fresh = tmp / "capsem-e2e-fresh"
    fresh.mkdir()
    # mtime is now; should not be removed.
    result = clean_stale.clean_tmp_fixtures(tmp, dry_run=False, verbose=False)

    assert result.removed == 0
    assert fresh.exists()


def test_tmp_fixture_non_matching_name_kept(tmp_path: Path):
    tmp = tmp_path / "T"
    tmp.mkdir()
    other = tmp / "unrelated-junk"
    other.mkdir()
    old_time = time.time() - 2 * 3600
    os.utime(other, (old_time, old_time))

    result = clean_stale.clean_tmp_fixtures(tmp, dry_run=False, verbose=False)

    assert result.removed == 0
    assert other.exists()


def test_cargo_prune_respects_threshold(tmp_path: Path):
    """Old deps files and old build/fingerprint/incremental dirs removed;
    recent ones kept. Use the moderate path (no release dir)."""
    debug = tmp_path / "target" / "debug"
    (debug / "deps").mkdir(parents=True)
    old_dep = debug / "deps" / "libold.rlib"
    new_dep = debug / "deps" / "libnew.rlib"
    old_dep.write_text("x")
    new_dep.write_text("y")

    old_time = time.time() - 10 * 86400  # 10 days ago
    os.utime(old_dep, (old_time, old_time))
    # new_dep has current mtime

    (debug / "build" / "crate-old").mkdir(parents=True)
    (debug / "build" / "crate-old" / "f").write_text("x")
    os.utime(debug / "build" / "crate-old", (old_time, old_time))

    (debug / "build" / "crate-new").mkdir(parents=True)
    (debug / "build" / "crate-new" / "f").write_text("x")

    (debug / ".fingerprint" / "stale").mkdir(parents=True)
    os.utime(debug / ".fingerprint" / "stale", (old_time, old_time))

    (debug / "incremental" / "stale").mkdir(parents=True)
    os.utime(debug / "incremental" / "stale", (old_time, old_time))

    result = clean_stale.clean_cargo_artifacts(tmp_path, dry_run=False, verbose=False)

    assert result.removed == 4
    assert not old_dep.exists()
    assert new_dep.exists()
    assert not (debug / "build" / "crate-old").exists()
    assert (debug / "build" / "crate-new").exists()
    assert not (debug / ".fingerprint" / "stale").exists()
    assert not (debug / "incremental" / "stale").exists()


def test_cargo_budget_evicts_oldest_incremental_over_cap(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """When incremental/ exceeds its size budget, oldest dirs get evicted.

    Regression: the age-based prune alone left 23 GB of incremental/ on disk
    during active dev because every build touches every session dir, so
    nothing ever crossed the 2-3 day age threshold. The size budget enforces
    a cap regardless of mtime freshness; oldest entries lose first.
    """
    # Shrink the budget so a realistic fixture exceeds it without needing GBs.
    monkeypatch.setitem(clean_stale.CARGO_KIND_BUDGETS_GB, "incremental", 0.000_01)  # ~10 KB
    incremental = tmp_path / "target" / "debug" / "incremental"
    incremental.mkdir(parents=True)
    # Three session dirs, each 8 KB. Combined 24 KB > 10 KB budget.
    for idx, age_days in enumerate([5, 2, 0]):  # oldest ... newest
        sess = incremental / f"s-{idx}"
        sess.mkdir()
        (sess / "blob").write_bytes(b"\x00" * 8192)
        t = time.time() - age_days * 86400
        os.utime(sess, (t, t))
    # Confirm nothing is "old" enough for the age-based prune (days <= 2 days ago
    # after aggressive threshold of 2d would prune s-0 only, leaving 16 KB > budget).
    result = clean_stale.clean_cargo_artifacts(tmp_path, dry_run=False, verbose=False)
    remaining = sorted(p.name for p in incremental.iterdir())
    # Budget pass must have evicted oldest (s-0, then s-1) until under 10 KB cap.
    # Newest s-2 (8 KB) must survive -- it's the warm cache we want to keep.
    assert "s-2" in remaining, f"newest session dir must survive, got {remaining}"
    assert "s-0" not in remaining, (
        f"oldest session dir must be evicted by budget, got {remaining}"
    )
    # Detail string should mention the budget pass fired.
    assert "budget=" in result.detail, (
        f"StageResult.detail should report budget evictions, got: {result.detail!r}"
    )


def test_cargo_budget_no_op_when_under_cap(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """With plenty of slack under the budget, the prune does nothing."""
    monkeypatch.setitem(clean_stale.CARGO_KIND_BUDGETS_GB, "incremental", 1.0)  # 1 GB cap
    incremental = tmp_path / "target" / "debug" / "incremental"
    incremental.mkdir(parents=True)
    sess = incremental / "fresh"
    sess.mkdir()
    (sess / "blob").write_bytes(b"\x00" * 1024)  # 1 KB, miles under budget

    result = clean_stale.clean_cargo_artifacts(tmp_path, dry_run=False, verbose=False)
    assert sess.exists(), "entry under budget must be kept"
    assert "budget=" not in result.detail, (
        f"no budget evictions expected, got: {result.detail!r}"
    )


def test_cargo_budget_deps_only_counts_cargo_extensions(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """Budget on deps/ must only touch cargo's .rlib/.o/.rmeta/.d output.

    Test binaries and other files (no extension, or extensions we don't
    manage) must survive even if the dir exceeds budget overall.
    """
    monkeypatch.setitem(clean_stale.CARGO_KIND_BUDGETS_GB, "deps", 0.000_01)  # ~10 KB
    deps = tmp_path / "target" / "debug" / "deps"
    deps.mkdir(parents=True)

    old_rlib = deps / "libcrate-aaa.rlib"
    old_rlib.write_bytes(b"\x00" * 8192)
    os.utime(old_rlib, (time.time() - 5 * 86400, time.time() - 5 * 86400))

    new_rlib = deps / "libcrate-bbb.rlib"
    new_rlib.write_bytes(b"\x00" * 8192)
    # Test binary -- no .rlib/.o/.rmeta/.d extension, must survive.
    test_bin = deps / "test_mycrate-abc123"
    test_bin.write_bytes(b"\x00" * 8192)
    os.utime(test_bin, (time.time() - 30 * 86400, time.time() - 30 * 86400))

    # Under the default age threshold (3 days), the ancient test_bin would be
    # targeted if the script counted it -- but the entry_filter scopes to
    # cargo extensions only, so the budget pass must leave it alone.
    clean_stale.clean_cargo_artifacts(tmp_path, dry_run=False, verbose=False)
    assert test_bin.exists(), "test binary must not be pruned by the budget pass"
    # Budget drops the oldest .rlib first -- old_rlib must be gone.
    assert not old_rlib.exists(), "oldest .rlib should have been evicted"


def test_cargo_prune_aggressive_drops_doc(tmp_path: Path):
    """When target/release has old content, aggressive mode is used and target/doc
    is dropped if nothing recent lives inside it."""
    release = tmp_path / "target" / "release"
    release.mkdir(parents=True)
    old_bin = release / "capsem"
    old_bin.write_text("x")
    old_time = time.time() - 5 * 86400
    os.utime(old_bin, (old_time, old_time))
    # Ensure release/ mtime itself is old so the heuristic triggers.
    os.utime(release, (old_time, old_time))

    doc = tmp_path / "target" / "doc"
    doc.mkdir(parents=True)
    (doc / "page.html").write_text("x")
    os.utime(doc / "page.html", (old_time, old_time))
    os.utime(doc, (old_time, old_time))

    result = clean_stale.clean_cargo_artifacts(tmp_path, dry_run=False, verbose=False)

    assert "aggressive" in result.detail
    assert not doc.exists()


def test_dry_run_removes_nothing(tmp_path: Path, short_sock_dir: Path):
    debug = tmp_path / "target" / "debug"
    debug.mkdir(parents=True)
    rootfs = debug / "rootfs.abc"
    rootfs.mkdir()

    sock = short_sock_dir / "dead.sock"
    _make_orphan_socket(sock)

    tmp = tmp_path / "T"
    tmp.mkdir()
    old = tmp / "capsem-test-foo"
    old.mkdir()
    old_time = time.time() - 2 * 3600
    os.utime(old, (old_time, old_time))

    # All stages with dry_run=True must keep files intact but still report counts.
    ra = clean_stale.clean_rootfs_scratch(tmp_path, dry_run=True, verbose=False)
    rb = clean_stale.clean_orphan_sockets(short_sock_dir, dry_run=True, verbose=False)
    rc = clean_stale.clean_tmp_fixtures(tmp, dry_run=True, verbose=False)

    assert ra.removed == 1 and rootfs.exists()
    assert rb.removed == 1 and sock.exists()
    assert rc.removed == 1 and old.exists()


def test_sockets_dir_missing(tmp_path: Path):
    """Missing sockets dir is not an error; returns zero removed."""
    result = clean_stale.clean_orphan_sockets(
        tmp_path / "does-not-exist", dry_run=False, verbose=False
    )
    assert result.removed == 0


def test_target_missing(tmp_path: Path):
    """Missing target/ dir is not an error for either stage."""
    ra = clean_stale.clean_rootfs_scratch(tmp_path, dry_run=False, verbose=False)
    rd = clean_stale.clean_cargo_artifacts(tmp_path, dry_run=False, verbose=False)
    assert ra.removed == 0
    assert rd.removed == 0
