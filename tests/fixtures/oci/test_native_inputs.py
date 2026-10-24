"""Original base descriptors and canonical workflow keys bind native candidates."""

import json
from pathlib import Path

import pytest
from capsem_builder.crypto import sha256

from tests.fixtures.oci import native_inputs

ROOT = Path(__file__).resolve().parents[3]


def oci(root, *, architecture="amd64", prefix=(), diff_prefix=()):
    blobs = root / "blobs/sha256"
    blobs.mkdir(parents=True)

    def blob(data, media):
        digest = "sha256:" + sha256(data).hexdigest()
        (blobs / digest[7:]).write_bytes(data)
        return {"mediaType": media, "digest": digest, "size": len(data)}

    layer = blob(
        b"new layer " + str(len(prefix)).encode(), "application/vnd.oci.image.layer.v1.tar+gzip"
    )
    diff = "sha256:" + sha256(b"uncompressed " + str(len(prefix)).encode()).hexdigest()
    config = {
        "os": "linux",
        "architecture": architecture,
        "rootfs": {"type": "layers", "diff_ids": [*diff_prefix, diff]},
    }
    descriptor = blob(json.dumps(config).encode(), "application/vnd.oci.image.config.v1+json")
    manifest = {
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": descriptor,
        "layers": [*prefix, layer],
    }
    for entry in prefix:
        (blobs / entry["digest"][7:]).write_bytes(b"new layer 0")
    subject = blob(json.dumps(manifest).encode(), manifest["mediaType"])
    (root / "index.json").write_text(json.dumps({"schemaVersion": 2, "manifests": [subject]}))
    return subject["digest"], manifest, config


def test_base_preserves_original_bytes_and_exact_ordered_child_prefix(tmp_path):
    base = tmp_path / "base"
    subject, manifest, config = oci(base)
    binding = native_inputs.BaseBinding.capture(
        base, subject, "linux/amd64", "a" * 64, maximum=1 << 20
    )
    assert binding.subject == subject
    assert binding.manifest_json.encode() == (base / "blobs/sha256" / subject[7:]).read_bytes()
    child = tmp_path / "child"
    digest, _, _ = oci(child, prefix=manifest["layers"], diff_prefix=config["rootfs"]["diff_ids"])
    binding.require_child(child, digest, "linux/amd64", maximum=1 << 20)
    restored = native_inputs.BaseBinding.model_validate_json(binding.model_dump_json())
    assert restored == binding
    restored.require_child(child, digest, "linux/amd64", maximum=1 << 20)


@pytest.mark.parametrize(
    "fault", ["platform", "compressed-prefix", "diff-prefix", "manifest", "config", "subject"]
)
def test_base_and_child_mismatch_cannot_be_attested(tmp_path, fault):
    base = tmp_path / "base"
    subject, manifest, config = oci(base)
    binding = native_inputs.BaseBinding.capture(
        base, subject, "linux/amd64", "a" * 64, maximum=1 << 20
    )
    prefix = manifest["layers"] if fault != "compressed-prefix" else []
    diffs = config["rootfs"]["diff_ids"] if fault != "diff-prefix" else ["sha256:" + "b" * 64]
    child = tmp_path / "child"
    digest, _, _ = oci(
        child,
        architecture="arm64" if fault == "platform" else "amd64",
        prefix=prefix,
        diff_prefix=diffs,
    )
    data = binding.model_dump()
    if fault in {"manifest", "config"}:
        data[fault + "_json"] += " "
    if fault == "subject":
        data["subject"] = "sha256:" + "f" * 64
    with pytest.raises(ValueError):
        native_inputs.BaseBinding.model_validate(data).require_child(
            child, digest, "linux/amd64", maximum=1 << 20
        )


def test_native_key_reads_the_actual_workflow_input_vector():
    from images.ci.inputkey import input_key

    patterns = [
        "images/ci/*.py",
        "build_system/builder/*.py",
        "build_system/builder/image/*.py",
        "build_system/builder/gate/*.py",
    ]
    fixed = [
        "images/codex-cli",
        ".github/workflows/images.yaml",
        "build_system/builder/release/obom.py",
        "build_system/builder/release/package_inventory.py",
        "build_system/docker/Dockerfile.asset-tools",
        "config/docker/image/build.toml",
        "config/gate.toml",
        "build_system/pyproject.toml",
        "build_system/uv.lock",
    ]
    inputs = [ROOT / path for path in fixed] + [
        path for pattern in patterns for path in ROOT.glob(pattern)
    ]
    base = "sha256:" + "c" * 64
    assert native_inputs.workflow_key(ROOT, "codex-cli", base) == input_key(
        inputs, root=ROOT, extra=[base]
    )
    assert native_inputs.workflow_key(ROOT, "base") == input_key([ROOT / "images/base"], root=ROOT)
    assert native_inputs.workflow_key(
        ROOT, "codex-cli", "sha256:" + "d" * 64
    ) != native_inputs.workflow_key(ROOT, "codex-cli", base)


@pytest.mark.parametrize("name", ["../foreign", "claude-desktop", "unknown"])
def test_workflow_key_is_not_an_arbitrary_recipe_selector(name):
    with pytest.raises(ValueError):
        native_inputs.workflow_key(ROOT, name, "sha256:" + "c" * 64)


@pytest.mark.parametrize("fault", ["link", "oversize", "changed-base-key", "changed-child-key"])
def test_original_metadata_and_current_inputs_are_required(monkeypatch, tmp_path, fault):
    base = tmp_path / "base"
    subject, manifest, config = oci(base)
    binding = native_inputs.BaseBinding.capture(
        base, subject, "linux/amd64", "a" * 64, maximum=1 << 20
    )
    child = tmp_path / "child"
    digest, _, _ = oci(child, prefix=manifest["layers"], diff_prefix=config["rootfs"]["diff_ids"])
    if fault == "link":
        path = child / "index.json"
        original = tmp_path / "original-index"
        path.rename(original)
        path.symlink_to(original)

    def key(root, name, base=None):
        return (
            "f"
            if fault == "changed-base-key" and name == "base"
            else "e"
            if fault == "changed-child-key" and name != "base"
            else "a"
            if name == "base"
            else "b"
        ) * 64

    monkeypatch.setattr(native_inputs, "workflow_key", key)
    with pytest.raises(ValueError):
        native_inputs.BuildBinding(input_key="b" * 64, base=binding).require(
            ROOT, "codex-cli", child, digest, "linux/amd64", 1 if fault == "oversize" else 1 << 20
        )


@pytest.mark.parametrize(
    "fault", ["index-size", "descriptor-path", "negative-size", "subject-path"]
)
def test_descriptor_admission_precedes_existing_blob_verification(tmp_path, fault):
    layout = tmp_path / "layout"
    subject, manifest, _ = oci(layout)
    if fault == "subject-path":
        subject = "sha256:../../outside"
    elif fault == "index-size":
        index = json.loads((layout / "index.json").read_bytes())
        index["manifests"][0]["size"] += 1
        (layout / "index.json").write_text(json.dumps(index))
    else:
        if fault == "descriptor-path":
            manifest["config"]["digest"] = "sha256:../../outside"
        else:
            manifest["layers"][0]["size"] = -1
        original = json.dumps(manifest).encode()
        subject = "sha256:" + sha256(original).hexdigest()
        (layout / "blobs/sha256" / subject[7:]).write_bytes(original)
        (layout / "index.json").write_text(
            json.dumps({"manifests": [{"digest": subject, "size": len(original)}]})
        )
    with pytest.raises(ValueError):
        native_inputs.metadata(layout, subject, "linux/amd64", 1048576)
