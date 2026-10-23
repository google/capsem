"""Producer proof and atomic staging failure regressions, without native execution."""

import json

import pytest

from tests.fixtures.oci import native_handoff as handoff
from tests.fixtures.oci.test_native_handoff import fixture


@pytest.mark.parametrize(
    "defect",
    ["missing", "failed", "torn", "wrong-command", "non-native", "foreign-source", "no-receipt"],
)
def test_producer_itself_must_have_completed_native_gate(monkeypatch, tmp_path, defect):
    config, _ = fixture(monkeypatch, tmp_path)
    path = config.path(config.runlog.root) / "build-0" / config.runlog.events
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    if defect == "missing":
        path.unlink()
    elif defect == "torn":
        path.write_text(path.read_text() + "{")
    else:
        if defect == "failed":
            rows[-1]["status"] = "failed"
        if defect == "wrong-command":
            rows[1]["argv"][-1] = "--foreign"
        if defect == "non-native":
            rows[0]["machine"] = "arm64"
        if defect == "foreign-source":
            rows[0]["head"] = "f" * 40
        if defect == "no-receipt":
            rows[0]["argv"].pop()
        path.write_text("".join(json.dumps(row) + "\n" for row in rows))
    with pytest.raises(ValueError):
        handoff.stage(config)
    assert not config.path(config.functional.native_images.artifact_dir).exists()


def test_failed_copy_never_publishes_partial_artifact(monkeypatch, tmp_path):
    config, _ = fixture(monkeypatch, tmp_path)

    def fail(*args):
        raise OSError("injected copy failure")

    monkeypatch.setattr(handoff, "copy_original", fail)
    with pytest.raises(OSError, match="copy failure"):
        handoff.stage(config)
    destination = config.path(config.functional.native_images.artifact_dir)
    assert not destination.exists()
    assert not list(destination.parent.glob(".native-stage-*"))


def test_handoff_uses_the_existing_real_generation_lease(monkeypatch, tmp_path):
    from types import SimpleNamespace

    from capsem_builder.cache.leases import active_path

    real_leased = handoff.leased
    config, selected = fixture(monkeypatch, tmp_path)
    image, receipt, layout, _ = selected["debug_image"]
    settings = image.settings()
    settings.cache_stage = "test-images"
    image.settings = lambda: settings
    paths = SimpleNamespace(
        stage=lambda stage: layout.parent,
        policy=SimpleNamespace(
            stages={"test-images": SimpleNamespace(lease_template=".{key}.lock")}
        ),
    )
    monkeypatch.setattr(handoff, "load_paths", lambda root: paths)
    lease = layout.parent / ("." + receipt.layout_key + ".lock")
    with real_leased(config, image) as (actual, found):
        assert actual == receipt and found == layout and active_path(lease)
    assert not active_path(lease)
