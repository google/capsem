"""OCI-layout and legacy `docker image save` archive ingestion into content-addressed blobs."""

from __future__ import annotations

import hashlib
import json
import os
import re
import tarfile
import uuid
from collections.abc import Mapping
from pathlib import Path
from typing import Any

_OCI_MANIFEST_MEDIA_TYPE = "application/vnd.oci.image.manifest.v1+json"
_DOCKER_MANIFEST_MEDIA_TYPE = "application/vnd.docker.distribution.manifest.v2+json"
_OCI_INDEX_MEDIA_TYPE = "application/vnd.oci.image.index.v1+json"
_DOCKER_LIST_MEDIA_TYPE = "application/vnd.docker.distribution.manifest.list.v2+json"
_SHA256_HEX_RE = re.compile(r"^[0-9a-f]{64}$")


def default_cache_dir() -> Path:
    """Return the root directory for cached OCI builds and local TLS certificates."""
    raw = os.environ.get("CAPSEM_INSPECT_BUILD_CACHE_DIR", "").strip()
    if raw:
        return Path(raw).expanduser().resolve()
    return (Path.home() / ".cache" / "capsem" / "inspect-oci-builds").resolve()


def _write_blob_bytes(blobs_dir: Path, data: bytes) -> str:
    hex_digest = hashlib.sha256(data).hexdigest()
    target = blobs_dir / hex_digest
    if not target.exists():
        tmp = blobs_dir / f".{hex_digest}.{os.getpid()}.{uuid.uuid4().hex}.tmp"
        tmp.write_bytes(data)
        tmp.replace(target)
    return hex_digest


def _resolve_tar_member(
    name: str, members: Mapping[str, bytes], blobs_dir: Path
) -> tuple[str, int]:
    norm = name.lstrip("./")
    if norm.startswith("blobs/sha256/"):
        hex_digest = norm.removeprefix("blobs/sha256/")
        blob_file = blobs_dir / hex_digest
        if blob_file.is_file():
            return hex_digest, blob_file.stat().st_size
    if norm in members:
        data = members[norm]
        return _write_blob_bytes(blobs_dir, data), len(data)
    raise RuntimeError(f"Missing referenced archive member {name!r} in docker save tar")


def _ingest_oci_layout_index(index_doc: Mapping[str, Any], blobs_dir: Path) -> str:
    manifests = index_doc.get("manifests")
    if not isinstance(manifests, list) or not manifests:
        raise RuntimeError("OCI index.json contains no manifests")
    chosen = next(
        (
            m
            for m in manifests
            if isinstance(m, Mapping)
            and (m.get("annotations") or {}).get("vnd.docker.reference.type")
            != "attestation-manifest"
        ),
        manifests[0],
    )
    digest = str(chosen.get("digest", "")).removeprefix("sha256:")
    if not _SHA256_HEX_RE.match(digest) or not (blobs_dir / digest).is_file():
        raise RuntimeError(f"OCI index.json points to missing manifest blob sha256:{digest}")
    media_type = str(chosen.get("mediaType", ""))
    blob_bytes = (blobs_dir / digest).read_bytes()
    doc = json.loads(blob_bytes.decode("utf-8"))
    if media_type in (_OCI_INDEX_MEDIA_TYPE, _DOCKER_LIST_MEDIA_TYPE) or "manifests" in doc:
        return _ingest_oci_layout_index(doc, blobs_dir)
    if doc.get("mediaType") == _DOCKER_MANIFEST_MEDIA_TYPE or "mediaType" not in doc:
        doc["mediaType"] = _OCI_MANIFEST_MEDIA_TYPE
        if isinstance(doc.get("config"), dict):
            doc["config"]["mediaType"] = "application/vnd.oci.image.config.v1+json"
        for layer in doc.get("layers") or []:
            if isinstance(layer, dict):
                mt = str(layer.get("mediaType", ""))
                layer["mediaType"] = (
                    "application/vnd.oci.image.layer.v1.tar+gzip"
                    if "gzip" in mt
                    else "application/vnd.oci.image.layer.v1.tar"
                )
        return _write_blob_bytes(blobs_dir, json.dumps(doc, separators=(",", ":")).encode("utf-8"))
    return digest


def _ingest_legacy_docker_manifest(
    manifest_list: Any, members: Mapping[str, bytes], blobs_dir: Path
) -> str:
    if not isinstance(manifest_list, list) or not manifest_list:
        raise RuntimeError("Invalid legacy docker save manifest.json")
    entry = manifest_list[0]
    cfg_hex, cfg_size = _resolve_tar_member(str(entry["Config"]), members, blobs_dir)
    layers: list[dict[str, Any]] = []
    for layer_path in entry.get("Layers") or []:
        l_str = str(layer_path)
        l_hex, l_size = _resolve_tar_member(l_str, members, blobs_dir)
        media_type = (
            "application/vnd.oci.image.layer.v1.tar+gzip"
            if l_str.endswith((".tar.gz", ".tgz"))
            else "application/vnd.oci.image.layer.v1.tar"
        )
        layers.append({"mediaType": media_type, "digest": f"sha256:{l_hex}", "size": l_size})
    oci_manifest = {
        "schemaVersion": 2,
        "mediaType": _OCI_MANIFEST_MEDIA_TYPE,
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": f"sha256:{cfg_hex}",
            "size": cfg_size,
        },
        "layers": layers,
    }
    return _write_blob_bytes(
        blobs_dir, json.dumps(oci_manifest, separators=(",", ":")).encode("utf-8")
    )


def ingest_docker_save_tar(tar_path: Path, cache_dir: Path | None = None) -> str:
    """Ingest a `docker image save` tar archive into `blobs/sha256` and return manifest hex."""
    c_dir = (cache_dir or default_cache_dir()).resolve()
    blobs_dir = c_dir / "blobs" / "sha256"
    blobs_dir.mkdir(parents=True, exist_ok=True)
    members: dict[str, bytes] = {}
    with tarfile.open(tar_path, "r:*") as tf:
        for member in tf.getmembers():
            if not member.isfile():
                continue
            fobj = tf.extractfile(member)
            if fobj is None:
                continue
            name = member.name.lstrip("./")
            data = fobj.read()
            if name.startswith("blobs/sha256/"):
                hex_part = name.removeprefix("blobs/sha256/")
                if _SHA256_HEX_RE.match(hex_part):
                    target = blobs_dir / hex_part
                    if not target.exists():
                        tmp = blobs_dir / f".{hex_part}.{os.getpid()}.{uuid.uuid4().hex}.tmp"
                        tmp.write_bytes(data)
                        tmp.replace(target)
                    continue
            members[name] = data

    if "index.json" in members:
        return _ingest_oci_layout_index(
            json.loads(members["index.json"].decode("utf-8")), blobs_dir
        )
    if "manifest.json" in members:
        return _ingest_legacy_docker_manifest(
            json.loads(members["manifest.json"].decode("utf-8")), members, blobs_dir
        )
    raise RuntimeError(
        f"Unsupported docker save archive {tar_path}: missing index.json and manifest.json"
    )


def _blob_valid(blobs_dir: Path, digest_or_hex: str) -> bool:
    hex_digest = digest_or_hex.removeprefix("sha256:")
    if len(hex_digest) != 64:
        return False
    blob_file = blobs_dir / hex_digest
    if not blob_file.is_file():
        return False
    if hashlib.sha256(blob_file.read_bytes()).hexdigest() != hex_digest:
        blob_file.unlink(missing_ok=True)
        return False
    return True


def verify_cached_manifest_blobs(blobs_dir: Path, manifest_hex: str) -> bool:
    """Verify manifest, config, and layer blobs exist and match their SHA-256 digests."""
    if not _blob_valid(blobs_dir, manifest_hex):
        return False
    try:
        doc = json.loads((blobs_dir / manifest_hex).read_bytes().decode("utf-8"))
        cfg_digest = str(doc["config"]["digest"])
        layer_digests = [str(layer["digest"]) for layer in (doc.get("layers") or [])]
    except Exception:
        return False
    cfg_ok = _blob_valid(blobs_dir, cfg_digest)
    layers_ok = [_blob_valid(blobs_dir, ld) for ld in layer_digests]
    return cfg_ok and all(layers_ok)
