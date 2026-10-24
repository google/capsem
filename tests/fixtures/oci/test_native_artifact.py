"""Actual synthetic descriptor/journal copies under isolated cache leases."""

import json
import shutil
from contextlib import nullcontext
from pathlib import Path

import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.leases import active_path
from capsem_builder.cache.paths import CachePaths

from tests.fixtures.oci import native_artifact, native_candidates, native_handoff
from tests.fixtures.oci.test_native_handoff import fixture

ROOT = Path(__file__).resolve().parents[3]


def prepared(monkeypatch, tmp_path):
    original_lease = native_handoff.leased
    config, selected = fixture(monkeypatch, tmp_path)
    cache = CachePaths(repository_root=tmp_path, policy=load_policy(ROOT))
    image, receipt, old_layout, _ = selected["debug_image"]
    stage = cache.stage("test-images")
    stage.mkdir(parents=True)
    layout = stage / (receipt.image + "-" + receipt.subject[7:])
    shutil.copytree(old_layout, layout)
    image.stage = lambda: stage
    image.layout_path = lambda _: layout
    settings = image.settings()
    settings.cache_stage = "test-images"
    image.settings = lambda: settings
    native_candidates.record(config, image, receipt.subject, receipt.platform, receipt.source)
    journal = config.path(config.runlog.root) / "run-0" / config.runlog.events
    rows = [json.loads(line) for line in journal.read_text().splitlines()]
    rows[0]["argv"][rows[0]["argv"].index("--layout") + 1] = str(layout)
    rows[1]["env"][config.functional.qualification.layout_variable] = str(layout)
    rows[1]["env"][config.functional.native_images.report_variable] = str(
        layout / config.functional.native_images.report_name
    )
    journal.write_text("".join(json.dumps(row) + "\n" for row in rows))
    monkeypatch.setattr(native_handoff, "leased", original_lease)
    monkeypatch.setattr(native_handoff, "load_paths", lambda _: cache)
    # This source fixture never acquires the real shared-machine lease. Its
    # separate ordering regression observes the actual prepare/copy boundary.
    monkeypatch.setattr(native_artifact, "machine", lambda _: nullcontext())
    return config, cache, image, receipt, layout


def test_original_bundle_is_copied_and_independently_verified_under_both_leases(
    monkeypatch, tmp_path
):
    config, cache, image, receipt, layout = prepared(monkeypatch, tmp_path)
    source_lock = layout.parent / ("." + layout.name + ".lock")
    with native_artifact.prepare(config, cache, image, "native-transfer-test") as (root, bundle):
        target_lock = root.parent / ("." + root.name + ".lock")
        assert active_path(source_lock) and active_path(target_lock)
        assert bundle.receipt.subject == receipt.subject
        shutil.copytree(root / "upload", root / "download")
        native_artifact.verify(config, root / "download", expected=bundle)
        assert active_path(source_lock) and active_path(target_lock)
    assert not active_path(source_lock) and not active_path(target_lock)
    assert (root / "upload/layout/index.json").read_bytes() == (layout / "index.json").read_bytes()


@pytest.mark.parametrize("fault", ["extra", "blob", "source", "journal", "link"])
def test_downloaded_artifact_cannot_claim_foreign_or_incomplete_original_proof(
    monkeypatch, tmp_path, fault
):
    config, cache, image, _, _ = prepared(monkeypatch, tmp_path)
    with native_artifact.prepare(config, cache, image, "native-transfer-test") as (root, bundle):
        target = root / "download"
        shutil.copytree(root / "upload", target)
        if fault == "extra":
            (target / "unlisted").write_text("not producer evidence")
        if fault == "blob":
            (target / "layout/index.json").write_text("{}")
        if fault == "source":
            data = json.loads((target / "handoff.json").read_bytes())
            data["receipt"]["source"]["commit"] = "f" * 40
            (target / "handoff.json").write_text(json.dumps(data))
        if fault == "journal":
            (target / "producer" / config.runlog.events).write_text("{}\n")
        if fault == "link":
            (target / "build.json").unlink()
            (target / "build.json").symlink_to(root / "upload/build.json")
        with pytest.raises((ValueError, KeyError)):
            native_artifact.verify(config, target, expected=bundle)


@pytest.mark.parametrize("field", ["config", "root", "root_integrity"])
def test_independent_receiver_binds_receipt_to_actual_original_descriptors(
    monkeypatch, tmp_path, field
):
    config, cache, image, _, _ = prepared(monkeypatch, tmp_path)
    with native_artifact.prepare(config, cache, image, "native-transfer-test") as (root, _):
        target = root / "download"
        shutil.copytree(root / "upload", target)
        metadata = json.loads((target / "handoff.json").read_bytes())
        receipt = json.loads((target / "build.json").read_bytes())
        receipt[field] = ("blake3:" if field == "root_integrity" else "sha256:") + "f" * 64
        metadata["receipt"] = receipt
        (target / "handoff.json").write_text(json.dumps(metadata))
        (target / "build.json").write_text(json.dumps(receipt))
        with pytest.raises(ValueError, match="descriptors"):
            native_artifact.verify(config, target)


def test_actual_cache_cleanup_preserves_both_live_generations_and_lease_inodes(
    monkeypatch, tmp_path
):
    from capsem_builder.cache.inventory import scan_inventory
    from capsem_builder.cache.operations import apply_prune
    from capsem_builder.cache.planner import plan_clean

    config, cache, image, _, layout = prepared(monkeypatch, tmp_path)
    source_lock = layout.parent / ("." + layout.name + ".lock")
    # The source was legitimately removable before the transfer took its lease.
    stale = plan_clean(
        scan_inventory(cache, cache.policy, stage_ids=frozenset({"test-images"})), "test-images"
    )
    with native_artifact.prepare(config, cache, image, "native-transfer-test") as (root, _):
        target_lock = root.parent / ("." + root.name + ".lock")
        before = source_lock.stat().st_ino, target_lock.stat().st_ino
        apply_prune(cache, stale, reason="isolated stale source transfer plan")
        inventory = scan_inventory(
            cache, cache.policy, stage_ids=frozenset({"test-images", "ironbank"})
        )
        cleanup = plan_clean(inventory, "all")
        apply_prune(cache, cleanup, reason="isolated live native transfer cleanup")
        assert before == (source_lock.stat().st_ino, target_lock.stat().st_ino)
        assert (layout / "index.json").is_file()
        assert (root / "upload/layout/index.json").read_bytes() == (
            layout / "index.json"
        ).read_bytes()
        assert active_path(source_lock) and active_path(target_lock)


def test_machine_lease_covers_projected_admission_and_copy_but_not_action_wait(
    monkeypatch, tmp_path
):
    from contextlib import contextmanager

    config, cache, image, _, layout = prepared(monkeypatch, tmp_path)
    active, copies = [], []
    original_copy = native_handoff.copy_original
    original_admit = native_artifact.native_transfer.enforce_repository

    @contextmanager
    def observed(_):
        active.append(True)
        try:
            yield
        finally:
            active.pop()

    def copy(*args):
        assert active == [True]
        copies.append(args[0])
        return original_copy(*args)

    def admit(*args, **kwargs):
        assert active == [True]
        return original_admit(*args, **kwargs)

    monkeypatch.setattr(native_artifact, "machine", observed)
    monkeypatch.setattr(native_handoff, "copy_original", copy)
    monkeypatch.setattr(native_artifact.native_transfer, "enforce_repository", admit)
    with native_artifact.prepare(config, cache, image, "native-transfer-machine") as (root, _):
        assert copies and not active
        assert active_path(layout.parent / ("." + layout.name + ".lock"))
        assert active_path(root.parent / ("." + root.name + ".lock"))
