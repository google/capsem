"""Tests for helpers.service.preserve_tmp_dir_on_failure.

Guards the artifact-capture filter rules. Previously the helper copied
everything in a tmp_dir on test failure; rootfs.img files and other
multi-GB blobs filled up the dev machine's disk (100% on /System/Volumes/Data).
These tests pin the skip list, the per-file size cap, and the rotation
so a future "helpful" loosening that reintroduces the bloat surfaces
in CI rather than on the next `just test` run.
"""

import os
from pathlib import Path

import pytest

# Import the module under test. Fixture below resets the module-level
# FAILED_NODEIDS / ARTIFACTS_ROOT state that the helper reads from
# tests.conftest.
from tests.helpers import service as svc_mod
from tests import conftest as tests_conftest


@pytest.fixture
def artifact_env(tmp_path, monkeypatch):
    """Point ARTIFACTS_ROOT at tmp_path and seed a single failed nodeid."""
    monkeypatch.setattr(tests_conftest, "ARTIFACTS_ROOT", tmp_path / "test-artifacts")
    # Replace, don't mutate -- other tests may run in the same process.
    monkeypatch.setattr(tests_conftest, "FAILED_NODEIDS", ["tests/fake/test_x.py::test_thing"])
    return tmp_path / "test-artifacts"


def _seed_tmp_dir(root: Path) -> Path:
    """Seed a tmp_dir mimicking what ServiceInstance produces."""
    tmp = root / "capsem-test-fixture"
    (tmp / "sessions" / "v1").mkdir(parents=True)
    (tmp / "logs").mkdir()
    # Files the helper SHOULD preserve.
    (tmp / "service.log").write_text("service logs\n")
    (tmp / "logs" / "gateway.log").write_text("gateway logs\n")
    (tmp / "sessions" / "v1" / "session.db").write_bytes(b"\x00" * 1024)
    (tmp / "sessions" / "v1" / "process.log").write_text("process logs\n")
    # Files the helper SHOULD skip.
    (tmp / "sessions" / "v1" / "rootfs.img").write_bytes(b"\x00" * (1024 * 1024))  # 1 MB
    (tmp / "sessions" / "v1" / "huge_blob.bin").write_bytes(
        b"\x00" * (svc_mod.ARTIFACT_MAX_FILE_BYTES + 1)
    )
    return tmp


def _copied_files(archive_root: Path) -> set[str]:
    """Return the set of file paths (relative) under the archive_root."""
    if not archive_root.exists():
        return set()
    return {
        str(p.relative_to(archive_root))
        for p in archive_root.rglob("*")
        if p.is_file()
    }


def test_rootfs_img_is_skipped(artifact_env, tmp_path):
    src = _seed_tmp_dir(tmp_path)
    svc_mod.preserve_tmp_dir_on_failure(src)

    copied = _copied_files(artifact_env)
    # Should NOT contain any rootfs.img.
    rootfs_copies = [p for p in copied if p.endswith("rootfs.img")]
    assert not rootfs_copies, (
        f"rootfs.img must never be archived; got: {rootfs_copies}"
    )


def test_oversize_files_are_skipped(artifact_env, tmp_path):
    src = _seed_tmp_dir(tmp_path)
    svc_mod.preserve_tmp_dir_on_failure(src)

    copied = _copied_files(artifact_env)
    huge_copies = [p for p in copied if p.endswith("huge_blob.bin")]
    assert not huge_copies, (
        f"files > ARTIFACT_MAX_FILE_BYTES must be skipped; got: {huge_copies}"
    )


def test_logs_and_session_db_are_preserved(artifact_env, tmp_path):
    src = _seed_tmp_dir(tmp_path)
    svc_mod.preserve_tmp_dir_on_failure(src)

    copied = _copied_files(artifact_env)
    # Should include logs and small session.db.
    expected = {"service.log", "logs/gateway.log", "sessions/v1/session.db",
                "sessions/v1/process.log"}
    for rel in expected:
        assert any(p.endswith(rel) for p in copied), (
            f"{rel} missing from archive (copied: {sorted(copied)})"
        )


def test_no_op_when_no_failures(artifact_env, tmp_path, monkeypatch):
    # Override artifact_env's FAILED_NODEIDS to be empty.
    monkeypatch.setattr(tests_conftest, "FAILED_NODEIDS", [])
    src = _seed_tmp_dir(tmp_path)
    svc_mod.preserve_tmp_dir_on_failure(src)
    assert not artifact_env.exists(), (
        "ARTIFACTS_ROOT should not be created when no tests failed"
    )


def test_rotation_keeps_only_most_recent_n(artifact_env, tmp_path, monkeypatch):
    # Shrink the cap for a fast test.
    monkeypatch.setattr(svc_mod, "ARTIFACT_MAX_KEPT_DIRS", 3)

    # Seed 5 pre-existing failure dirs with timestamps.
    artifact_env.mkdir(parents=True)
    for i, stamp in enumerate([
        "20260101-000001-gw0-fail-1",
        "20260101-000002-gw0-fail-2",
        "20260101-000003-gw0-fail-3",
        "20260101-000004-gw0-fail-4",
        "20260101-000005-gw0-fail-5",
    ]):
        (artifact_env / stamp).mkdir()
        (artifact_env / stamp / "marker").write_text(str(i))

    # Trigger a preserve, which runs rotation.
    src = _seed_tmp_dir(tmp_path)
    svc_mod.preserve_tmp_dir_on_failure(src)

    remaining = sorted(p.name for p in artifact_env.iterdir() if p.is_dir())
    # We pass cap=3, add 1 new dir -> expect the 3 newest to survive.
    # Newest: the freshly-preserved one + two of the pre-seeded "fail-4", "fail-5".
    assert len(remaining) == 3, f"expected 3 dirs after rotation, got {remaining}"
    assert "20260101-000001-gw0-fail-1" not in remaining
    assert "20260101-000002-gw0-fail-2" not in remaining
    assert "20260101-000005-gw0-fail-5" in remaining
    assert "20260101-000004-gw0-fail-4" in remaining
