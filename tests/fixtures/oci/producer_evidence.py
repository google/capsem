"""Original canonical producer bytes retained beside their immutable subject.

The caller owns the existing cache-generation/build lease. This is local fixture
provenance, not another scanner, BOM format, release manifest or publication rail.
OCI descriptors reuse the existing SHA-256 wire-format/integrity owner.
"""

from __future__ import annotations

import json
import os
import stat
import tempfile
from pathlib import Path

from . import published_root
from .catalog_evidence import INVENTORY, OBOM, read_evidence
from .pinned_image import verify

FILES = {"packages.json": INVENTORY, "obom.cdx.json": OBOM, "rootfs.blake3": "text/plain"}
DIRECTORY = "producer-evidence"


def regular_files(root, *, maximum=None):
    """Stream no-follow directory entries; a bound is enforced during enumeration."""
    if ".." in root.parts:
        raise ValueError("producer evidence refuses path traversal")
    if root.is_symlink() or not root.is_dir():
        raise ValueError("producer evidence root must be a real directory")
    if maximum is not None and (type(maximum) is not int or maximum <= 0):
        raise ValueError("producer evidence entry budget invalid")
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    count = 0

    def walk(descriptor, relative):
        nonlocal count
        with os.scandir(descriptor) as entries:
            for entry in entries:
                count += 1
                if maximum is not None and count > maximum:
                    raise ValueError("producer evidence entry budget exceeded")
                mode = entry.stat(follow_symlinks=False).st_mode
                child = relative / entry.name
                if stat.S_ISDIR(mode):
                    opened = os.open(entry.name, flags, dir_fd=descriptor)
                    try:
                        yield from walk(opened, child)
                    finally:
                        os.close(opened)
                elif stat.S_ISREG(mode):
                    yield root / child
                else:
                    raise ValueError("producer evidence refuses symlink or special file")

    descriptor = os.open(root, flags)
    try:
        yield from walk(descriptor, Path())
    finally:
        os.close(descriptor)


def safe_tree(root):
    """Leased host-owned generations contain only actual directories/files."""
    for _ in regular_files(root):
        pass


def read_file(path, maximum):
    if maximum <= 0:
        raise ValueError("producer evidence budget exceeded")
    # Do not follow even the final component if it changes after tree checking.
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or not 0 < info.st_size <= maximum:
            raise ValueError("producer evidence file budget/type invalid")
        data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError("producer evidence budget exceeded")
    return data


def metadata(output, maximum):
    result = {}
    remaining = maximum
    for name in FILES:
        data = read_file(output / name, remaining)
        remaining -= len(data)
        result[name] = data
    return result


def ready(layout, repository, architecture, *, maximum):
    """Validate original metadata, platform and the exact retained root blob."""
    safe_tree(layout)
    directory = layout / DIRECTORY
    safe_tree(directory)
    if {path.name for path in directory.iterdir()} != {*FILES, "receipt.json"}:
        raise ValueError("producer evidence files differ from the allowlist")
    receipt = json.loads(read_file(directory / "receipt.json", maximum))
    if set(receipt) != {"subject", "repository", "platform", "root", "files"}:
        raise ValueError("producer evidence receipt shape differs")
    subject = published_root.subject(layout)
    if (receipt["subject"], receipt["repository"], receipt["platform"]) != (
        subject,
        repository,
        "linux/" + architecture,
    ):
        raise ValueError("producer evidence subject/repository/platform differs")
    verify(layout, subject["digest"], "linux/" + architecture)
    root, blob = published_root.ready(layout)
    if receipt["root"] != root["layers"][0]:
        raise ValueError("producer evidence original root differs")
    original = metadata(directory, maximum)
    expected = {
        name: published_root.descriptor(data, FILES[name]) for name, data in original.items()
    }
    if receipt["files"] != expected:
        raise ValueError("producer evidence original metadata bytes differ")
    if (
        original["rootfs.blake3"].decode().strip()
        != root["annotations"]["org.capsem.rootfs.blake3"]
    ):
        raise ValueError("producer evidence original root integrity differs")
    read_evidence(directory, repository + "@" + subject["digest"], architecture, subject["digest"])
    assert blob.is_file()  # existing root owner validates bytes; no second EROFS copy
    return directory


def receipt(subject, root, original, repository, architecture):
    return {
        "subject": subject,
        "repository": repository,
        "platform": "linux/" + architecture,
        "root": root["layers"][0],
        "files": {
            name: published_root.descriptor(data, FILES[name]) for name, data in original.items()
        },
    }


def required_bytes(subject, output, repository, architecture, *, maximum):
    """Exact original root+metadata write peak, including atomic temporary trees.

    Both temporary trees rename into place; they are not copied a second time.
    Count the full root even when the existing writer can hard-link it.
    """
    safe_tree(output)
    original = metadata(output, maximum)
    read_evidence(output, repository + "@" + subject["digest"], architecture, subject["digest"])
    root, files = published_root.metadata(subject, output / "rootfs.erofs")
    if (
        original["rootfs.blake3"].decode().strip()
        != root["annotations"]["org.capsem.rootfs.blake3"]
    ):
        raise ValueError("producer original root integrity differs")
    encoded = json.dumps(
        receipt(subject, root, original, repository, architecture), sort_keys=True
    ).encode()
    if len(encoded) > maximum:
        raise ValueError("producer evidence receipt budget exceeded")
    return (
        root["layers"][0]["size"]
        + sum(map(len, files.values()))
        + sum(map(len, original.values()))
        + len(encoded)
    )


def stage(layout, output, repository, architecture, *, maximum):
    """Retain first producer output atomically; never replace it with a repack."""
    safe_tree(layout)
    safe_tree(output)
    subject = published_root.subject(layout)
    verify(layout, subject["digest"], "linux/" + architecture)
    original = metadata(output, maximum)
    read_evidence(output, repository + "@" + subject["digest"], architecture, subject["digest"])
    directory = layout / DIRECTORY
    if directory.exists():
        ready(layout, repository, architecture, maximum=maximum)
        if any((directory / name).read_bytes() != data for name, data in original.items()):
            raise ValueError("original producer metadata differs; refusing regeneration")
        from capsem_builder.crypto import digest_file

        if original["rootfs.blake3"].decode().strip() != "blake3:" + digest_file(
            output / "rootfs.erofs"
        ):
            raise ValueError("original root differs from regenerated bytes")
        return directory
    if (layout / "rootfs-v2").exists():
        raise ValueError(
            "retained root is missing original producer evidence; refusing recovery repack"
        )
    root, _ = published_root.stage(layout, output)
    if (
        original["rootfs.blake3"].decode().strip()
        != root["annotations"]["org.capsem.rootfs.blake3"]
    ):
        raise ValueError("producer original root integrity differs")
    recorded = receipt(subject, root, original, repository, architecture)
    with tempfile.TemporaryDirectory(dir=layout, prefix=".evidence-") as temporary:
        staged = Path(temporary) / DIRECTORY
        staged.mkdir()
        for name, data in {
            **original,
            "receipt.json": json.dumps(recorded, sort_keys=True).encode(),
        }.items():
            if len(data) > maximum:
                raise ValueError("producer evidence receipt budget exceeded")
            path = staged / name
            path.write_bytes(data)
            path.chmod(0o600)
        staged.rename(directory)
    return directory
