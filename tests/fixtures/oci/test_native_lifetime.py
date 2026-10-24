"""Real synthetic bytes/leases with a thread standing in for the CLI process."""

import os
import shutil
from concurrent.futures import ThreadPoolExecutor

import pytest
from capsem_builder.cache.leases import active_path

from tests.fixtures.oci import native_artifact, native_lifetime, native_owner
from tests.fixtures.oci.test_native_artifact import prepared


def setup(monkeypatch, tmp_path, *, hold_seconds=5):
    config, paths, image, _, _ = prepared(monkeypatch, tmp_path)
    transfer = config.functional.native_images.transfer.model_copy(
        update={
            "poll_seconds": 0.001,
            "ready_seconds": 2,
            "release_seconds": 2,
            "hold_seconds": hold_seconds,
        }
    )
    native = config.functional.native_images.model_copy(update={"transfer": transfer})
    config = config.model_copy(
        update={"functional": config.functional.model_copy(update={"native_images": native})}
    )
    monkeypatch.setattr(native_lifetime, "load_paths", lambda _: paths)
    monkeypatch.setattr(native_lifetime, "image", lambda *_: image)
    monkeypatch.setattr(native_owner, "lock_holders", lambda _: (f"pid {os.getpid()} (fixture)",))
    return config, paths


def test_original_bundle_owner_survives_upload_download_until_verified_release(
    monkeypatch, tmp_path
):
    config, paths = setup(monkeypatch, tmp_path)
    with ThreadPoolExecutor(max_workers=1) as executor:
        running = executor.submit(native_lifetime.hold, config, "debug_image", "10", "1")
        owner = native_lifetime.ready(config, "debug_image", "10", "1")
        root = native_lifetime.root(config, paths, owner)
        lease = native_owner.lease(config, paths, owner)
        assert active_path(lease)
        with pytest.raises((FileNotFoundError, ValueError)):
            native_lifetime.settle(config, "debug_image", "10", "1", artifact_id="23")
        assert active_path(lease) and not running.done()
        shutil.copytree(root / "upload", root / "download")
        native_artifact.verify(config, root / "download", expected=owner.bundle)
        result = native_lifetime.settle(config, "debug_image", "10", "1", artifact_id="23")
        running.result(timeout=2)
        assert result.request.artifact_id == "23" and not active_path(lease)
        assert (root / "upload/layout/index.json").exists()


def test_abort_after_failed_consumer_releases_only_own_leases_without_success(
    monkeypatch, tmp_path
):
    config, paths = setup(monkeypatch, tmp_path)
    with ThreadPoolExecutor(max_workers=1) as executor:
        running = executor.submit(native_lifetime.hold, config, "debug_image", "10", "1")
        owner = native_lifetime.ready(config, "debug_image", "10", "1")
        result = native_lifetime.settle(config, "debug_image", "10", "1", abort=True)
        running.result(timeout=2)
        assert result.request.action == "abort" and result.request.artifact_id is None
        assert not active_path(native_owner.lease(config, paths, owner))
        with pytest.raises((FileNotFoundError, ValueError)):
            native_lifetime.finish(config, "10", "1")


def test_expired_owner_does_not_publish_success_or_accept_late_release(monkeypatch, tmp_path):
    config, _ = setup(monkeypatch, tmp_path, hold_seconds=0.03)
    with ThreadPoolExecutor(max_workers=1) as executor:
        running = executor.submit(native_lifetime.hold, config, "debug_image", "10", "1")
        with pytest.raises(TimeoutError):
            running.result(timeout=2)
        key = native_owner.key(config, "debug_image", "10", "1")
        assert not native_owner.control(config, key, "complete").exists()
        with pytest.raises(ValueError):
            native_lifetime.settle(config, "debug_image", "10", "1", abort=True)


def test_base_owner_retains_only_original_producer_generation_without_rebuilding(
    monkeypatch, tmp_path
):
    from tests.fixtures.oci import native_candidates
    from tests.fixtures.oci.native_inputs import BaseBinding, BuildBinding
    from tests.fixtures.oci.test_native_inputs import oci

    config, paths = setup(monkeypatch, tmp_path)
    generation = paths.stage("test-images") / ("native-base-amd64-" + "a" * 64)
    layout = generation / "layout"
    digest, _, _ = oci(layout)
    binding = BaseBinding.capture(layout, digest, "linux/amd64", "a" * 64, maximum=1 << 20)
    selected = native_candidates.Receipt.model_validate_json(
        native_candidates.receipt_path(config, "debug_image").read_bytes()
    ).model_copy(
        update={
            "section": "reference_image",
            "build": BuildBinding(input_key="b" * 64, base=binding),
        }
    )
    native_candidates.receipt_path(config, "reference_image").write_text(selected.model_dump_json())
    monkeypatch.setattr(native_lifetime, "workflow_key", lambda *_: "a" * 64)
    before = (layout / "index.json").read_bytes()
    lease = generation.parent / ("." + generation.name + ".lock")
    with native_lifetime.base(config, paths) as retained:
        assert retained == binding and active_path(lease)
        assert (layout / "index.json").read_bytes() == before
    assert not active_path(lease)
    (layout / "blobs/sha256" / binding.subject[7:]).write_bytes(b"wrong original bytes")
    with pytest.raises(ValueError), native_lifetime.base(config, paths):
        pytest.fail("foreign base was accepted")
    assert not active_path(lease)
