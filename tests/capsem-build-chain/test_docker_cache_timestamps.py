"""Real Docker projection of a valid OCI image with no creation timestamp."""

import hashlib
import io
import json
import subprocess
import tarfile
import uuid
from collections.abc import Iterator
from pathlib import Path

import pytest
from capsem_builder.cache import dockeradapter
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.runtimemodels import DockerRuntimePolicy


@pytest.fixture
def timestamp_less_image() -> Iterator[tuple[DockerRuntimePolicy, str]]:
    tag = f"capsem-cache-created-test-{uuid.uuid4().hex}:fixture"
    layer = io.BytesIO()
    with tarfile.open(fileobj=layer, mode="w") as archive:
        payload = b"cache metadata fixture\n"
        entry = tarfile.TarInfo("fixture.txt")
        entry.size = len(payload)
        archive.addfile(entry, io.BytesIO(payload))
    # Docker's external archive format requires JSON and SHA-256 layer IDs.
    # OCI creation time is optional; intentionally leave it absent.
    config = json.dumps({
        "architecture": "amd64", "os": "linux",
        "config": {"Labels": {"test.owner": tag}},
        "rootfs": {"type": "layers", "diff_ids": [
            "sha256:" + hashlib.sha256(layer.getvalue()).hexdigest(),
        ]},
    }).encode()
    manifest = json.dumps([
        {"Config": "config.json", "RepoTags": [tag], "Layers": ["layer.tar"]},
    ]).encode()
    image = io.BytesIO()
    with tarfile.open(fileobj=image, mode="w") as archive:
        for name, data in (("config.json", config), ("layer.tar", layer.getvalue()),
                           ("manifest.json", manifest)):
            entry = tarfile.TarInfo(name)
            entry.size = len(data)
            archive.addfile(entry, io.BytesIO(data))
    policy = load_policy(Path(__file__).resolve().parents[2]).runtimes["docker"]
    assert isinstance(policy, DockerRuntimePolicy)
    subprocess.run([policy.command, "image", "load"], input=image.getvalue(),
                   capture_output=True, check=True, timeout=30)
    try:
        yield policy.model_copy(update={"image_prefixes": (tag.split(":")[0],)}), tag
    finally:
        # This fixture created exactly this unique tag; no cache is pruned.
        subprocess.run([policy.command, "image", "rm", tag],
                       capture_output=True, check=True, timeout=30)


@pytest.mark.parametrize("consumer", ["inventory", "exact-anchor"])
def test_timestamp_less_docker_image_remains_accounted(
    timestamp_less_image: tuple[DockerRuntimePolicy, str], consumer: str,
) -> None:
    policy, tag = timestamp_less_image
    if consumer == "inventory":
        report = dockeradapter.inventory("docker", policy)
        assert report.available, report.error
        resource = next(row for row in report.resources if tag in row.names)
        assert report.owned_bytes >= resource.logical_bytes
    else:
        resource = dockeradapter.inspect_image(policy, tag)
        assert resource is not None, "a valid timestamp-less image is not absent"
        assert resource.protected
    assert resource.names == (tag,)
    assert resource.owned and resource.logical_bytes > 0
    assert resource.created_ns == resource.last_used_ns == 0, "retain the unknown-age sentinel"
