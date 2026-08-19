#!/usr/bin/env python3
"""Read the files out of a built package, whatever archive shape it uses.

Split out of `check-public-binary-release.py`, which is far past the size a
first-party script may be and could not grow to gain a liveness check until
something left it. This is the part with nothing to do with releases: `.deb` is
`ar` around a compressed tar, `.pkg` is `xar` around a cpio, and unpacking
either is archive plumbing that a release checker should be able to call
without containing.
"""

from __future__ import annotations

import gzip
import shutil
import subprocess
import tarfile
import tempfile
import zlib
from io import BytesIO
from pathlib import Path


def package_payload_files(package_path: Path) -> dict[str, bytes]:
    if package_path.name.endswith(".deb"):
        return deb_payload_files(package_path)
    if package_path.name.endswith(".pkg"):
        return pkg_payload_files(package_path)
    return {}


def deb_payload_files(package_path: Path) -> dict[str, bytes]:
    contents = package_path.read_bytes()
    offset = 8
    if not contents.startswith(b"!<arch>\n"):
        raise ValueError("invalid ar header")
    data_member: bytes | None = None
    data_member_name = "data.tar"
    while offset + 60 <= len(contents):
        header = contents[offset : offset + 60]
        name = header[:16].decode("ascii", errors="replace").strip().rstrip("/")
        size = int(header[48:58].decode("ascii").strip())
        data_start = offset + 60
        data_end = data_start + size
        data = contents[data_start:data_end]
        if name.startswith("data.tar"):
            data_member = data
            data_member_name = name
            break
        offset = data_end + (size % 2)
    if data_member is None:
        raise ValueError("missing data.tar member")
    return tar_payload_files(data_member, data_member_name)


def tar_payload_files(payload: bytes, member_name: str) -> dict[str, bytes]:
    try:
        with tarfile.open(fileobj=BytesIO(payload), mode="r:*") as archive:
            rows: dict[str, bytes] = {}
            for member in archive.getmembers():
                if not member.isfile():
                    continue
                handle = archive.extractfile(member)
                if handle is None:
                    continue
                rows[normalize_payload_path(member.name)] = handle.read()
            return rows
    except tarfile.TarError:
        with tempfile.TemporaryDirectory() as raw_tmp:
            raw = Path(raw_tmp)
            archive_path = raw / member_name
            payload_dir = raw / "payload"
            archive_path.write_bytes(payload)
            payload_dir.mkdir()
            subprocess.run(
                ["tar", "xf", str(archive_path.resolve()), "-C", str(payload_dir)],
                check=True,
                capture_output=True,
            )
            return {
                normalize_payload_path(path.relative_to(payload_dir).as_posix()): path.read_bytes()
                for path in payload_dir.rglob("*")
                if path.is_file()
            }


def pkg_payload_files(package_path: Path) -> dict[str, bytes]:
    if shutil.which("pkgutil") is not None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            expanded = Path(raw_tmp) / "expanded"
            subprocess.run(
                ["pkgutil", "--expand-full", str(package_path.resolve()), str(expanded)],
                check=True,
                capture_output=True,
            )
            rows: dict[str, bytes] = {}
            for payload in [path for path in expanded.rglob("Payload") if path.is_dir()]:
                for path in payload.rglob("*"):
                    if path.is_file():
                        rows[normalize_payload_path(path.relative_to(payload).as_posix())] = (
                            path.read_bytes()
                        )
            return rows
    return xar_pkg_payload_files(package_path)


def xar_pkg_payload_files(package_path: Path) -> dict[str, bytes]:
    contents = package_path.read_bytes()
    if len(contents) < 28 or contents[:4] != b"xar!":
        raise ValueError("not a xar .pkg archive")
    header_size = int.from_bytes(contents[4:6], "big")
    compressed_toc_size = int.from_bytes(contents[8:16], "big")
    toc_end = header_size + compressed_toc_size
    if header_size < 28 or toc_end > len(contents):
        raise ValueError("invalid xar header")
    toc = zlib.decompress(contents[header_size:toc_end]).decode("utf-8")
    rows: dict[str, bytes] = {}
    search_from = 0
    while True:
        name_index = toc.find("<name>Payload</name>", search_from)
        if name_index < 0:
            break
        block_start = toc.rfind("<file", 0, name_index)
        block_end = toc.find("</file>", name_index)
        if block_start < 0 or block_end < 0:
            raise ValueError("malformed Payload metadata")
        block = toc[block_start : block_end + len("</file>")]
        offset = int(xml_tag(block, "offset"))
        length = int(xml_tag(block, "length"))
        payload = contents[toc_end + offset : toc_end + offset + length]
        if len(payload) != length:
            raise ValueError("truncated Payload")
        if "application/x-gzip" in block or payload.startswith(b"\x1f\x8b"):
            payload = gzip.decompress(payload)
        rows.update(cpio_payload_files(payload))
        search_from = block_end + len("</file>")
    return rows


def xml_tag(block: str, tag: str) -> str:
    start = block.find(f"<{tag}>")
    end = block.find(f"</{tag}>", start)
    if start < 0 or end < 0:
        raise ValueError(f"xar Payload metadata missing {tag}")
    return block[start + len(tag) + 2 : end].strip()


def cpio_payload_files(payload: bytes) -> dict[str, bytes]:
    if payload.startswith(b"070707"):
        return odc_cpio_payload_files(payload)
    rows: dict[str, bytes] = {}
    offset = 0
    while offset < len(payload):
        header = payload[offset : offset + 110]
        if len(header) < 110:
            raise ValueError("newc cpio header truncated")
        if header[:6] not in {b"070701", b"070702"}:
            raise ValueError("newc cpio header magic mismatch")
        mode = int(header[14:22], 16)
        file_size = int(header[54:62], 16)
        name_size = int(header[94:102], 16)
        name_start = offset + 110
        name_end = name_start + name_size
        name = payload[name_start : name_end - 1].decode("utf-8")
        data_start = align4(name_end)
        data_end = data_start + file_size
        if name == "TRAILER!!!":
            break
        if mode & 0o170000 == 0o100000:
            rows[normalize_payload_path(name)] = payload[data_start:data_end]
        offset = align4(data_end)
    return rows


def odc_cpio_payload_files(payload: bytes) -> dict[str, bytes]:
    rows: dict[str, bytes] = {}
    offset = 0
    while offset < len(payload):
        header = payload[offset : offset + 76]
        if len(header) < 76:
            raise ValueError("odc cpio header truncated")
        if header[:6] != b"070707":
            raise ValueError("odc cpio header magic mismatch")
        mode = int(header[18:24], 8)
        name_size = int(header[59:65], 8)
        file_size = int(header[65:76], 8)
        name_start = offset + 76
        name_end = name_start + name_size
        name = payload[name_start : name_end - 1].decode("utf-8")
        data_start = name_end
        data_end = data_start + file_size
        if name == "TRAILER!!!":
            break
        if mode & 0o170000 == 0o100000:
            rows[normalize_payload_path(name)] = payload[data_start:data_end]
        offset = data_end
    return rows


def align4(value: int) -> int:
    return (value + 3) & ~3


def normalize_payload_path(path: str) -> str:
    return "/" + path.removeprefix("./").lstrip("/")
