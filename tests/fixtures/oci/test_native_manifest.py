"""Six synthetic completed consumers cannot hide source/base/artifact mismatch."""

import time

import pytest

from tests.fixtures.oci import native_artifact, native_manifest, native_owner
from tests.fixtures.oci.native_inputs import BaseBinding, BuildBinding
from tests.fixtures.oci.test_native_artifact import prepared
from tests.fixtures.oci.test_native_inputs import oci


def completed(monkeypatch, tmp_path):
    config, paths, _, receipt, layout = prepared(monkeypatch, tmp_path)
    base_layout = tmp_path / "synthetic-base"
    subject, _, _ = oci(base_layout)
    binding = BaseBinding.capture(base_layout, subject, "linux/amd64", "a" * 64, maximum=1 << 20)
    monkeypatch.setattr(native_manifest, "load_paths", lambda _: paths)
    monkeypatch.setattr(
        native_manifest,
        "workflow_key",
        lambda _, name, base=None: ("a" if name == "base" else "b") * 64,
    )
    rows = []
    for index, section in enumerate((*native_owner.SECTIONS, "base")):
        key = native_owner.key(config, section, "10", "1")
        selected = receipt.model_copy(
            update={
                "section": section,
                "image": "dev"
                if section == "reference_image"
                else "capsem-debug"
                if section == "debug_image"
                else section,
                "build": None
                if section in ("debug_image", "base")
                else BuildBinding(input_key="b" * 64, base=binding),
            }
        )
        bundle = (
            None
            if section == "base"
            else native_artifact.Bundle(
                receipt=selected, source_root=config.root, original_layout=layout
            )
        )
        owner = native_owner.Owner.model_validate(
            {
                "key": key,
                "section": section,
                "source": receipt.source,
                "pid": 9999999,
                "created": 1,
                "nonce": "7a48e229-223f-4d21-a293-08f1f8ac767a",
                "expires": time.time() + 100,
                "device": 1,
                "inode": 2,
                "bundle": bundle,
                "base": binding if section == "base" else None,
            }
        )
        request = native_owner.Request(
            owner=owner, action="release", artifact_id=None if section == "base" else str(index + 1)
        )
        value = native_owner.Completion(request=request, finished=time.time())
        path = native_owner.control(config, key, "complete")
        native_owner.publish(path, value, config.functional.native_images.receipt_max_bytes)
        rows.append((path, value))
    return config, rows


def test_exact_six_original_consumers_complete_one_shared_base_manifest(monkeypatch, tmp_path):
    config, _ = completed(monkeypatch, tmp_path)
    result = native_manifest.finish(config, "10", "1")
    assert len(result["candidates"]) == 6
    assert len({row["artifact_id"] for row in result["candidates"]}) == 6
    assert result["platform"] == "linux/amd64"


@pytest.mark.parametrize(
    "fault",
    [
        "missing",
        "duplicate-id",
        "source",
        "platform",
        "abort",
        "expiry",
        "base",
        "input",
        "base-abort",
    ],
)
def test_partial_foreign_or_mixed_base_consumers_cannot_complete(monkeypatch, tmp_path, fault):
    config, rows = completed(monkeypatch, tmp_path)
    path, result = rows[-1 if fault.startswith("base") else 1]
    data = result.model_dump(mode="json")
    owner = data["request"]["owner"]
    if fault == "missing":
        path.unlink()
    else:
        if fault == "duplicate-id":
            data["request"]["artifact_id"] = "1"
        if fault == "source":
            owner["source"]["commit"] = "f" * 40
        if fault == "platform":
            owner["bundle"]["receipt"]["platform"] = "linux/arm64"
        if fault in ("abort", "base-abort"):
            data["request"]["action"], data["request"]["artifact_id"] = "abort", None
        if fault == "expiry":
            data["finished"] = owner["expires"] + 1
        if fault == "base":
            owner["base"]["input_key"] = "f" * 64
        if fault == "input":
            owner["bundle"]["receipt"]["build"]["input_key"] = "f" * 64
        path.write_text(native_owner.Completion.model_validate(data).model_dump_json())
    with pytest.raises((ValueError, FileNotFoundError)):
        native_manifest.finish(config, "10", "1")
