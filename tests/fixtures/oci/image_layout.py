"""Small deterministic OCI layers for unpacking and registry fixtures."""

import gzip
import hashlib
import io
import json
import tarfile
from pathlib import Path


def _layer(files, links=()):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for name, data in files:
            entry = tarfile.TarInfo(name)
            entry.mode = 0o644
            entry.size = len(data)
            archive.addfile(entry, io.BytesIO(data))
        for name, target in links:
            entry = tarfile.TarInfo(name)
            entry.type = tarfile.SYMTYPE
            entry.mode = 0o777
            entry.linkname = target
            archive.addfile(entry)
    return output.getvalue()


def write_layout(root: Path, architecture: str):
    """Create a standard OCI layout; never unpack archive data on the host."""
    blobs = root / "blobs" / "sha256"
    blobs.mkdir(parents=True)

    def blob(data, kind):
        digest = hashlib.sha256(data).hexdigest()
        (blobs / digest).write_bytes(data)
        return {"mediaType": kind, "digest": f"sha256:{digest}", "size": len(data)}

    layers = [
        _layer(
            [("keep", b"kept"), ("delete", b"deleted"), ("dir/old", b"old")],
            [("link", "/keep")],
        ),
        _layer(
            [
                (".wh.delete", b""),
                ("dir/.wh..wh..opq", b""),
                ("dir/new", bytes(range(256))),
            ]
        ),
    ]
    config = {
        "architecture": architecture,
        "os": "linux",
        "config": {"Entrypoint": ["/bin/app"], "Cmd": ["serve"]},
        "rootfs": {
            "type": "layers",
            "diff_ids": [f"sha256:{hashlib.sha256(data).hexdigest()}" for data in layers],
        },
    }
    manifest = {
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": blob(json.dumps(config).encode(), "application/vnd.oci.image.config.v1+json"),
        "layers": [
            blob(gzip.compress(data, mtime=0), "application/vnd.oci.image.layer.v1.tar+gzip")
            for data in layers
        ],
    }
    descriptor = blob(json.dumps(manifest).encode(), manifest["mediaType"])
    descriptor["annotations"] = {"org.opencontainers.image.ref.name": "image"}
    (root / "index.json").write_text(json.dumps({"schemaVersion": 2, "manifests": [descriptor]}))
    (root / "oci-layout").write_text('{"imageLayoutVersion":"1.0.0"}')
    return descriptor["digest"]
