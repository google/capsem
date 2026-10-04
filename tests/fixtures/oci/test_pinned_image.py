"""capsem-debug's fixture: only the pinned bytes are ever served."""

import hashlib
import json
import ssl

import pytest

from tests.fixtures.oci.pinned_image import debug_image, reference_image
from tests.fixtures.oci.registry import serve

MANIFEST = "application/vnd.oci.image.manifest.v1+json"


def _layout(root, *, architecture="arm64", media=MANIFEST):
    """A one-layer OCI layout; returns (layout, manifest digest, blobs by digest)."""
    blobs = {}

    def blob(data, kind):
        digest = "sha256:" + hashlib.sha256(data).hexdigest()
        blobs[digest] = data
        return {"mediaType": kind, "digest": digest, "size": len(data)}

    config = blob(
        json.dumps({"os": "linux", "architecture": architecture}).encode(), "config"
    )
    layer = blob(b"layer bytes", "application/vnd.oci.image.layer.v1.tar+gzip")
    manifest = json.dumps(
        {"schemaVersion": 2, "mediaType": media, "config": config, "layers": [layer]}
    ).encode()
    digest = blob(manifest, media)["digest"]
    layout = root / "layout"
    (layout / "blobs" / "sha256").mkdir(parents=True)
    for name, data in blobs.items():
        (layout / "blobs" / "sha256" / name.removeprefix("sha256:")).write_bytes(data)
    entry = {"mediaType": media, "digest": digest, "size": len(manifest)}
    (layout / "index.json").write_text(
        json.dumps({"schemaVersion": 2, "manifests": [entry]})
    )
    return layout, digest, blobs


def test_an_intact_layout_of_the_pinned_platform_verifies(tmp_path):
    layout, digest, _ = _layout(tmp_path)
    debug_image.verify(layout, digest, "linux/arm64")


@pytest.mark.parametrize(
    "defect", ["tampered layer", "missing layer", "other digest", "other platform"]
)
def test_anything_but_the_pinned_bytes_is_refused(tmp_path, defect):
    layout, digest, blobs = _layout(tmp_path)
    layer = next(name for name, data in blobs.items() if data == b"layer bytes")
    path = layout / "blobs" / "sha256" / layer.removeprefix("sha256:")
    platform = "linux/arm64"
    if defect == "tampered layer":
        path.write_bytes(b"layer bytez")
    elif defect == "missing layer":
        path.unlink()
    elif defect == "other digest":
        digest = "sha256:" + "0" * 64
    else:
        platform = "linux/amd64"
    with pytest.raises(ValueError):
        debug_image.verify(layout, digest, platform)


def test_an_index_is_not_a_pin(tmp_path):
    """A pin names one platform's manifest; an index could resolve to another."""
    layout, digest, _ = _layout(
        tmp_path, media="application/vnd.oci.image.index.v1+json"
    )
    with pytest.raises(ValueError, match="not one platform"):
        debug_image.verify(layout, digest, "linux/arm64")


def test_an_unpinned_platform_names_what_is_pinned():
    with pytest.raises(ValueError, match="pins no image for linux/s390x"):
        debug_image.pinned("linux/s390x")


@pytest.mark.parametrize(
    "image", [debug_image, reference_image], ids=lambda image: image.section
)
def test_every_pin_is_a_manifest_digest_and_lands_in_its_own_entry(image):
    for platform, digest in image.settings().digests.items():
        assert image.pinned(platform) == digest
        assert (
            image.layout_path(digest).name
            == f"{image.settings().name}-{digest.removeprefix('sha256:')}"
        )
        assert image.layout_path(digest).parent == image.stage()


def test_the_reference_image_is_the_official_dev_image_built_on_base():
    """Runtime tests boot the product's own shape, not a fixture: the official
    `dev` image, from its own Dockerfile and on the base every image shares."""
    settings = reference_image.settings()
    assert (settings.name, settings.context, settings.base_context) == (
        "dev",
        "images/dev",
        "images/base",
    )
    assert settings.repository.endswith("/dev")


def _pull(tmp_path, fetch):
    _, digest, blobs = _layout(tmp_path / "source")
    manifest = blobs[digest]
    with serve(tmp_path, "capsem-debug", manifest, MANIFEST, fetch(blobs)) as (
        reference,
        ca,
        requests,
    ):
        target = tmp_path / "pulled"
        debug_image.pull(
            digest,
            "linux/arm64",
            target,
            repository=reference.split("@")[0],
            context=ssl.create_default_context(cafile=str(ca)),
        )
    return target, digest, requests


def test_a_pull_by_digest_keeps_the_exact_bytes(tmp_path):
    target, digest, requests = _pull(tmp_path, lambda blobs: blobs.get)
    debug_image.verify(target, digest, "linux/arm64")
    assert f"/v2/library/capsem-debug/manifests/{digest}" in requests
    assert not any(path.endswith(":latest") or "/tags/" in path for path in requests)


def test_a_registry_serving_other_bytes_is_refused_and_leaves_nothing(tmp_path):
    def lying(blobs):
        return lambda digest: (
            b"not the layer"
            if blobs.get(digest) == b"layer bytes"
            else blobs.get(digest)
        )

    with pytest.raises(ValueError, match="did not serve"):
        _pull(tmp_path, lying)
    assert not (tmp_path / "pulled").exists()
    assert not list(tmp_path.glob(".pull-*"))
