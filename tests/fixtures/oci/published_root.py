"""Prepared EROFS artifacts beside pinned OCI layouts, bound to their subject."""

from __future__ import annotations

import json
import os
import re
import shutil
import tempfile
import urllib.error
from pathlib import Path

from capsem_builder import crypto as hashlib
from capsem_builder.crypto import digest_file

MEDIA = "application/vnd.capsem.rootfs.erofs.v2"
MANIFEST = "application/vnd.oci.image.manifest.v1+json"
EMPTY = "application/vnd.oci.empty.v1+json"
DIGEST = re.compile(r"sha256:[0-9a-f]{64}")
LIMIT = 4 * 1024**3


def sha256(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1 << 20), b""):
            value.update(chunk)
    return "sha256:" + value.hexdigest()


def descriptor(data: bytes, kind: str) -> dict:
    return {
        "mediaType": kind,
        "digest": "sha256:" + hashlib.sha256(data).hexdigest(),
        "size": len(data),
    }


def subject(layout: Path) -> dict:
    (entry,) = json.loads((layout / "index.json").read_text())["manifests"]
    return {key: entry[key] for key in ("mediaType", "digest", "size")}


def _blob(root: Path, digest: str) -> Path:
    if not DIGEST.fullmatch(digest):
        raise ValueError(f"invalid prepared-root digest: {digest}")
    return root / "blobs/sha256" / digest.removeprefix("sha256:")


def _verified(root: Path, entry: dict, kind: str, limit: int) -> Path:
    path = _blob(root, entry["digest"])
    if (
        entry["mediaType"] != kind
        or not 0 < entry["size"] <= limit
        or path.stat().st_size != entry["size"]
        or sha256(path) != entry["digest"]
    ):
        raise ValueError("prepared-root bytes or descriptor differ")
    return path


def ready(layout: Path, *, root: Path | None = None) -> tuple[dict, Path]:
    root = root or layout / "rootfs-v2"
    (entry,) = json.loads((root / "index.json").read_text())["manifests"]
    manifest_path = _verified(root, entry, MANIFEST, 4 * 1024 * 1024)
    manifest = json.loads(manifest_path.read_bytes())
    if (
        manifest["schemaVersion"] != 2
        or manifest["artifactType"] != MEDIA
        or manifest["subject"] != subject(layout)
    ):
        raise ValueError("prepared-root subject differs from the platform image")
    config = _verified(root, manifest["config"], EMPTY, 2)
    if config.read_bytes() != b"{}":
        raise ValueError("prepared-root config is not empty")
    (layer,) = manifest["layers"]
    path = _verified(root, layer, MEDIA, LIMIT)
    integrity = manifest.get("annotations", {}).get("org.capsem.rootfs.blake3")
    if integrity != "blake3:" + digest_file(path):
        raise ValueError("prepared-root BLAKE3 integrity differs")
    return manifest, path


def stage(layout: Path, output: Path) -> tuple[dict, Path]:
    """Retain canonical producer output; reject an OBOM for another image."""
    image = subject(layout)
    component = json.loads((output / "obom.cdx.json").read_text())["metadata"]["component"]
    if component["version"] != image["digest"]:
        raise ValueError("producer subject differs from the platform image")
    filesystem = output / "rootfs.erofs"
    size = filesystem.stat().st_size
    if not 0 < size <= LIMIT:
        raise ValueError("prepared-root size exceeds the runtime bound")
    root_digest = sha256(filesystem)
    with tempfile.TemporaryDirectory(dir=layout, prefix=".rootfs-") as temporary:
        root = Path(temporary) / "rootfs-v2"
        (root / "blobs/sha256").mkdir(parents=True)
        try:
            os.link(filesystem, _blob(root, root_digest))
        except OSError:
            shutil.copyfile(filesystem, _blob(root, root_digest))
        manifest = {
            "schemaVersion": 2,
            "mediaType": MANIFEST,
            "artifactType": MEDIA,
            "subject": image,
            "config": descriptor(b"{}", EMPTY),
            "layers": [{"mediaType": MEDIA, "digest": root_digest, "size": size}],
            "annotations": {"org.capsem.rootfs.blake3": "blake3:" + digest_file(filesystem)},
        }
        data = json.dumps(manifest, sort_keys=True).encode()
        _blob(root, manifest["config"]["digest"]).write_bytes(b"{}")
        entry = descriptor(data, MANIFEST)
        _blob(root, entry["digest"]).write_bytes(data)
        (root / "index.json").write_text(json.dumps({"schemaVersion": 2, "manifests": [entry]}))
        (root / "oci-layout").write_text('{"imageLayoutVersion":"1.0.0"}')
        if (layout / "rootfs-v2").exists():
            existing, _ = ready(layout)
            if existing['layers'][0]['digest'] != root_digest:
                raise ValueError('prepared original root differs from producer output')
        else:
            root.rename(layout / "rootfs-v2")
    return ready(layout)


def _download(client, url, token, root, entry, kind, limit):
    if entry["mediaType"] != kind or not 0 < entry["size"] <= limit:
        raise ValueError(
            "published prepared-root descriptor exceeds the bound or has another media type"
        )
    path = _blob(root, entry["digest"])
    count = 0
    with client.open(url, token, kind) as response, path.open("wb") as target:
        while chunk := response.read(min(1 << 20, entry["size"] - count + 1)):
            count += len(chunk)
            if count > entry["size"]:
                raise ValueError("published prepared-root bytes exceed the descriptor")
            target.write(chunk)
    _verified(root, entry, kind, limit)


def prepare(layout: Path, repository: str, *, context=None) -> None:
    """Fetch the published subject-bound artifact once, before offline tests."""
    if (layout / "rootfs-v2").exists():
        ready(layout)
        return
    from tests.fixtures.oci.pinned_image import _Registry, _token

    registry, name = repository.split("/", 1)
    client = _Registry(context)
    token = _token(client, registry, name)
    base = f"https://{registry}/v2/{name}"
    digest = subject(layout)["digest"]
    try:
        response = client.open(f"{base}/referrers/{digest}", token)
    except urllib.error.HTTPError as error:
        with error:
            if error.code != 404:
                raise
            # OCI's fallback index is required by registries without the referrers API.
            response = client.open(
                f"{base}/manifests/{digest.replace(':', '-', 1)}",
                token,
                "application/vnd.oci.image.index.v1+json",
            )
    with response:
        data = response.read(4 * 1024 * 1024 + 1)
        if len(data) > 4 * 1024 * 1024:
            raise ValueError("published prepared-root referrers exceed the metadata bound")
        entries = json.loads(data)["manifests"]
    candidates = [entry for entry in entries if entry.get("artifactType") == MEDIA]
    if len(candidates) != 1:
        raise ValueError(
            f"{repository}: expected one published EROFS artifact, found {len(candidates)}"
        )
    (entry,) = candidates
    with tempfile.TemporaryDirectory(dir=layout, prefix=".rootfs-") as temporary:
        root = Path(temporary) / "rootfs-v2"
        (root / "blobs/sha256").mkdir(parents=True)
        _download(
            client,
            f"{base}/manifests/{entry['digest']}",
            token,
            root,
            entry,
            MANIFEST,
            4 * 1024 * 1024,
        )
        manifest = json.loads(_blob(root, entry["digest"]).read_bytes())
        if manifest["subject"] != subject(layout):
            raise ValueError("published prepared-root subject differs")
        if manifest["schemaVersion"] != 2 or manifest["artifactType"] != MEDIA:
            raise ValueError("unsupported prepared-root artifact")
        (layer,) = manifest["layers"]
        for blob, kind, limit in (
            (manifest["config"], EMPTY, 2),
            (layer, MEDIA, LIMIT),
        ):
            _download(
                client,
                f"{base}/blobs/{blob['digest']}",
                token,
                root,
                blob,
                kind,
                limit,
            )
        (root / "index.json").write_text(
            json.dumps(
                {
                    "schemaVersion": 2,
                    "manifests": [{key: entry[key] for key in ("mediaType", "digest", "size")}],
                }
            )
        )
        (root / "oci-layout").write_text('{"imageLayoutVersion":"1.0.0"}')
        ready(layout, root=root)
        root.rename(layout / "rootfs-v2")
    ready(layout)
