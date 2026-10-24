"""Synthetic cross-step receipts; no image construction or upload is performed."""

import os
import time

import psutil
import pytest
from capsem_builder.cache.leases import release_path, retain_generation

from tests.fixtures.oci import native_owner
from tests.fixtures.oci.native_candidates import Source
from tests.fixtures.oci.test_native_artifact import prepared


def ticket(monkeypatch, tmp_path):
    config, paths, _, receipt, _ = prepared(monkeypatch, tmp_path)
    key = native_owner.key(config, "debug_image", "12", "3")
    root = paths.stage("ironbank") / key
    root.mkdir(parents=True)
    descriptor = retain_generation(paths, "ironbank", key)
    lease = paths.stage("ironbank") / ("." + key + ".lock")
    info = os.fstat(descriptor.fileno())
    owner = native_owner.Owner(
        key=key,
        source=receipt.source,
        section="debug_image",
        pid=os.getpid(),
        created=psutil.Process().create_time(),
        nonce="7a48e229-223f-4d21-a293-08f1f8ac767a",
        expires=time.time() + 100,
        device=info.st_dev,
        inode=info.st_ino,
    )
    monkeypatch.setattr(
        native_owner, "lock_holders", lambda path: (f"pid {os.getpid()} (fixture)",)
    )
    return config, paths, root, lease, owner


def test_live_owner_binds_actual_held_inode_and_process_generation(monkeypatch, tmp_path):
    config, paths, _, lease, owner = ticket(monkeypatch, tmp_path)
    try:
        native_owner.require_live(config, paths, owner)
        release_path(lease)
        with pytest.raises(ValueError, match="lease"):
            native_owner.require_live(config, paths, owner)
    finally:
        release_path(lease)


@pytest.mark.parametrize(
    "fault", ["created", "pid", "inode", "device", "expires", "source", "holder"]
)
def test_stale_foreign_or_unlocked_owner_cannot_authorize_action(monkeypatch, tmp_path, fault):
    config, paths, _, lease, owner = ticket(monkeypatch, tmp_path)
    updates = {
        "created": 1.0,
        "pid": 99999999,
        "inode": 0,
        "device": -1,
        "expires": time.time() - 1,
        "source": Source(commit="f" * 40, tree="f" * 40),
    }
    if fault == "holder":
        monkeypatch.setattr(native_owner, "lock_holders", lambda path: ("pid 1 (foreign)",))
    else:
        owner = owner.model_copy(update={fault: updates[fault]})
    try:
        with pytest.raises(ValueError):
            native_owner.require_live(config, paths, owner)
    finally:
        release_path(lease)


def test_control_is_bounded_exact_and_cannot_overwrite(monkeypatch, tmp_path):
    config, _, _, lease, owner = ticket(monkeypatch, tmp_path)
    path = native_owner.control(config, owner.key, "ready")
    try:
        native_owner.publish(path, owner, config.functional.native_images.receipt_max_bytes)
        assert native_owner.read(config, path, native_owner.Owner) == owner
        with pytest.raises(FileExistsError):
            native_owner.publish(path, owner, config.functional.native_images.receipt_max_bytes)
        with pytest.raises(ValueError):
            native_owner.publish(path.with_name("oversize"), owner, 1)
        path.unlink()
        path.symlink_to(tmp_path / "private")
        with pytest.raises((ValueError, OSError)):
            native_owner.read(config, path, native_owner.Owner)
    finally:
        release_path(lease)


def test_release_signal_cannot_target_newer_nonce_or_claim_missing_download(monkeypatch, tmp_path):
    _config, _, _, lease, owner = ticket(monkeypatch, tmp_path)
    try:
        request = native_owner.Request(
            owner=owner, action="release", artifact_id="18446744073709551615"
        )
        assert native_owner.accept(owner, request) == "18446744073709551615"
        with pytest.raises(ValueError):
            native_owner.accept(
                owner.model_copy(update={"nonce": "77c812bb-89ab-4d1c-9ae0-00bd3eb08e33"}), request
            )
        for artifact_id in ("0", "-1", "18446744073709551616", "3\n", "abc"):
            with pytest.raises(ValueError):
                native_owner.Request(owner=owner, action="release", artifact_id=artifact_id)
        with pytest.raises(ValueError):
            native_owner.Request(owner=owner, action="release")
        with pytest.raises(ValueError):
            native_owner.Request(owner=owner, action="abort", artifact_id="1")
    finally:
        release_path(lease)


def test_final_manifest_requires_six_unique_ids_one_source_and_common_original_base(tmp_path):
    # Full official BuildBindings are covered by native artifact/proof tests.
    rows = []
    for index, section in enumerate(native_owner.SECTIONS):
        rows.append({"section": section, "artifact_id": str(index + 1)})
    assert len(native_owner.complete_sections(rows)) == 6
    for broken in (rows[:-1], rows + rows[:1], [*rows[:-1], {**rows[-1], "artifact_id": "1"}]):
        with pytest.raises(ValueError):
            native_owner.complete_sections(broken)
