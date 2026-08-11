#!/usr/bin/env python3
"""Download, verify and safely unpack one ort-sys static distribution."""

from __future__ import annotations

import argparse
import hashlib
import lzma
import shutil
import tarfile
import tempfile
import urllib.request
from pathlib import Path, PurePosixPath

MAX_ARCHIVE_BYTES = 1 << 30
MAX_EXPANDED_BYTES = 8 << 30
MAX_ENTRIES = 100_000
CHUNK = 1 << 20


def _arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def _safe_name(name: str) -> Path:
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError(f"ORT archive entry escapes its root: {name}")
    return Path(*path.parts)


def _download(url: str, expected: str, destination: Path) -> None:
    digest = hashlib.sha256()
    total = 0
    with urllib.request.urlopen(url, timeout=1800) as response, destination.open("wb") as output:
        while chunk := response.read(CHUNK):
            total += len(chunk)
            if total > MAX_ARCHIVE_BYTES:
                raise ValueError("ORT archive exceeds the 1 GiB limit")
            digest.update(chunk)
            output.write(chunk)
    actual = digest.hexdigest()
    if actual != expected:
        raise ValueError(f"ORT archive digest mismatch: got {actual}, expected {expected}")
    print(f"verified ORT archive sha256={actual} bytes={total}")


def _decompress(source: Path, destination: Path) -> None:
    decompressor = lzma.LZMADecompressor(
        format=lzma.FORMAT_RAW,
        filters=[{"id": lzma.FILTER_LZMA2, "dict_size": 1 << 26}],
    )
    total = 0
    with source.open("rb") as archive, destination.open("wb") as output:
        while chunk := archive.read(CHUNK):
            expanded = decompressor.decompress(chunk)
            total += len(expanded)
            if total > MAX_EXPANDED_BYTES:
                raise ValueError("ORT archive expands beyond the 8 GiB limit")
            output.write(expanded)
        if not decompressor.eof:
            raise ValueError("ORT archive ended before the LZMA2 stream")


def _extract(source: Path, output: Path) -> None:
    output.mkdir(parents=True, exist_ok=False)
    entries = 0
    total = 0
    with tarfile.open(source, mode="r:") as archive:
        for member in archive:
            entries += 1
            if entries > MAX_ENTRIES:
                raise ValueError("ORT archive contains too many entries")
            relative = _safe_name(member.name)
            destination = output / relative
            if member.isdir():
                destination.mkdir(parents=True, exist_ok=True)
                continue
            if not member.isfile():
                raise ValueError(f"ORT archive has unsupported entry: {member.name}")
            total += member.size
            if total > MAX_EXPANDED_BYTES:
                raise ValueError("ORT archive files exceed the 8 GiB limit")
            destination.parent.mkdir(parents=True, exist_ok=True)
            extracted = archive.extractfile(member)
            if extracted is None:
                raise ValueError(f"ORT archive entry has no bytes: {member.name}")
            with destination.open("xb") as target:
                copied = shutil.copyfileobj(extracted, target, CHUNK)
            if copied is not None:
                raise AssertionError("copyfileobj unexpectedly returned a value")
            if destination.stat().st_size != member.size:
                raise ValueError(f"ORT archive entry was truncated: {member.name}")
            destination.chmod(0o444)
    library = output / "libonnxruntime.a"
    if not library.is_file():
        raise ValueError("ORT archive has no root libonnxruntime.a")
    print(f"materialized static ORT library at {library}")


def main() -> None:
    args = _arguments()
    if args.output.exists():
        raise ValueError(f"ORT output already exists: {args.output}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="capsem-ort-") as temporary:
        root = Path(temporary)
        compressed = root / "ort.tar.lzma2"
        tar = root / "ort.tar"
        _download(args.url, args.sha256, compressed)
        _decompress(compressed, tar)
        _extract(tar, args.output)


if __name__ == "__main__":
    main()
