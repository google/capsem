#!/usr/bin/env python3
"""Generate a minimal SPDX SBOM for packaged Capsem host binaries."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import zlib
from pathlib import Path


EXECUTABLE_PREFIXES = (
    "usr/bin/",
    "usr/local/bin/",
    "usr/local/share/capsem/bin/",
    "Applications/Capsem.app/Contents/MacOS/",
)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("artifacts", nargs="+")
    args = parser.parse_args()

    files = []
    for artifact in [Path(item) for item in args.artifacts]:
        files.extend(executable_entries(artifact))
    files.sort(key=lambda item: (item["fileName"], item["SPDXID"]))

    document = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": "capsem-host-binaries",
        "documentNamespace": "https://release.capsem.org/sbom/capsem-host-binaries",
        "creationInfo": {
            "creators": ["Tool: capsem generate-host-binary-sbom"],
            "created": "2026-07-03T00:00:00Z",
        },
        "files": files,
    }
    Path(args.output).write_text(json.dumps(document, indent=2, sort_keys=True) + "\n")
    return 0


def executable_entries(artifact: Path) -> list[dict[str, object]]:
    if artifact.name.endswith(".deb"):
        return deb_entries(artifact)
    if artifact.name.endswith(".pkg"):
        return pkg_entries(artifact)
    return []


def deb_entries(artifact: Path) -> list[dict[str, object]]:
    data_name, data_contents = deb_data_member(artifact)
    with tempfile.TemporaryDirectory() as raw_tmp:
        raw = Path(raw_tmp)
        data = raw / data_name
        data.write_bytes(data_contents)
        payload = raw / "payload"
        payload.mkdir()
        subprocess.run(["tar", "xf", str(data.resolve()), "-C", str(payload)], check=True)
        return entries_from_payload(payload)


def deb_data_member(artifact: Path) -> tuple[str, bytes]:
    """Read a Debian package's data archive without platform-specific `ar`."""
    contents = artifact.read_bytes()
    if not contents.startswith(b"!<arch>\n"):
        raise SystemExit(f"{artifact} is not an ar archive")

    offset = 8
    while offset + 60 <= len(contents):
        header = contents[offset : offset + 60]
        if header[58:60] != b"`\n":
            raise SystemExit(f"{artifact} has an invalid ar member header")
        name = header[:16].decode("ascii", errors="replace").strip().rstrip("/")
        try:
            size = int(header[48:58].decode("ascii").strip())
        except ValueError as error:
            raise SystemExit(f"{artifact} has an invalid ar member size") from error
        data_start = offset + 60
        data_end = data_start + size
        if data_end > len(contents):
            raise SystemExit(f"{artifact} has a truncated ar member")
        if name.startswith("data.tar"):
            return name, contents[data_start:data_end]
        offset = data_end + (size % 2)

    raise SystemExit(f"{artifact} has no data.tar payload")


def pkg_entries(artifact: Path) -> list[dict[str, object]]:
    if shutil.which("pkgutil") is None:
        return xar_pkg_entries(artifact)
    with tempfile.TemporaryDirectory() as raw_tmp:
        raw = Path(raw_tmp)
        expanded = raw / "expanded"
        subprocess.run(["pkgutil", "--expand-full", str(artifact.resolve()), str(expanded)], check=True)
        payloads = [path for path in expanded.rglob("Payload") if path.is_dir()]
        entries: list[dict[str, object]] = []
        for payload in payloads:
            entries.extend(entries_from_payload(payload))
        return entries


def xar_pkg_entries(artifact: Path) -> list[dict[str, object]]:
    contents = artifact.read_bytes()
    if len(contents) < 28 or contents[:4] != b"xar!":
        raise SystemExit(f"{artifact} is not a xar .pkg archive")
    header_size = int.from_bytes(contents[4:6], "big")
    compressed_toc_size = int.from_bytes(contents[8:16], "big")
    toc_end = header_size + compressed_toc_size
    if header_size < 28 or toc_end > len(contents):
        raise SystemExit(f"{artifact} has an invalid xar header")
    toc = zlib.decompress(contents[header_size:toc_end]).decode("utf-8")
    rows: list[dict[str, object]] = []
    search_from = 0
    while True:
        name_index = toc.find("<name>Payload</name>", search_from)
        if name_index < 0:
            break
        block_start = toc.rfind("<file", 0, name_index)
        block_end = toc.find("</file>", name_index)
        if block_start < 0 or block_end < 0:
            raise SystemExit(f"{artifact} has malformed Payload metadata")
        block = toc[block_start : block_end + len("</file>")]
        offset = int(_xml_tag(block, "offset"))
        length = int(_xml_tag(block, "length"))
        payload = contents[toc_end + offset : toc_end + offset + length]
        if len(payload) != length:
            raise SystemExit(f"{artifact} Payload is truncated")
        if "application/x-gzip" in block or payload.startswith(b"\x1f\x8b"):
            payload = gzip.decompress(payload)
        rows.extend(entries_from_newc_payload(payload))
        search_from = block_end + len("</file>")
    if not rows:
        raise SystemExit(f"{artifact} contains no Capsem executable Payload entries")
    return rows


def _xml_tag(block: str, tag: str) -> str:
    start = block.find(f"<{tag}>")
    end = block.find(f"</{tag}>", start)
    if start < 0 or end < 0:
        raise SystemExit(f"xar Payload metadata missing {tag}")
    return block[start + len(tag) + 2 : end].strip()


def entries_from_newc_payload(payload: bytes) -> list[dict[str, object]]:
    if payload.startswith(b"070707"):
        return entries_from_odc_payload(payload)
    rows: list[dict[str, object]] = []
    offset = 0
    while offset < len(payload):
        header = payload[offset : offset + 110]
        if len(header) < 110:
            raise SystemExit("newc cpio header truncated")
        if header[:6] not in {b"070701", b"070702"}:
            raise SystemExit("newc cpio header magic mismatch")
        mode = int(header[14:22], 16)
        file_size = int(header[54:62], 16)
        name_size = int(header[94:102], 16)
        name_start = offset + 110
        name_end = name_start + name_size
        name = payload[name_start : name_end - 1].decode("utf-8")
        data_start = _align4(name_end)
        data_end = data_start + file_size
        if name == "TRAILER!!!":
            break
        if mode & 0o170000 == 0o100000 and mode & 0o111:
            relative = name.removeprefix("./")
            if relative.startswith(EXECUTABLE_PREFIXES):
                rows.append(spdx_file_entry(relative, payload[data_start:data_end]))
        offset = _align4(data_end)
    return rows


def entries_from_odc_payload(payload: bytes) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    offset = 0
    while offset < len(payload):
        header = payload[offset : offset + 76]
        if len(header) < 76:
            raise SystemExit("odc cpio header truncated")
        if header[:6] != b"070707":
            raise SystemExit("odc cpio header magic mismatch")
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
        if mode & 0o170000 == 0o100000 and mode & 0o111:
            relative = name.removeprefix("./")
            if relative.startswith(EXECUTABLE_PREFIXES):
                rows.append(spdx_file_entry(relative, payload[data_start:data_end]))
        offset = data_end
    return rows


def _align4(value: int) -> int:
    return (value + 3) & ~3


def entries_from_payload(payload: Path) -> list[dict[str, object]]:
    rows = []
    for path in payload.rglob("*"):
        if not path.is_file() or not os.access(path, os.X_OK):
            continue
        relative = path.relative_to(payload).as_posix().removeprefix("./")
        if not relative.startswith(EXECUTABLE_PREFIXES):
            continue
        rows.append(spdx_file_entry(relative, path.read_bytes()))
    return rows


def spdx_file_entry(relative: str, contents: bytes) -> dict[str, object]:
    name = Path(relative).name
    return {
        "SPDXID": f"SPDXRef-File-{spdx_fragment(name)}",
        "fileName": f"/{relative}",
        "checksums": [
            {
                "algorithm": "SHA256",
                "checksumValue": hashlib.sha256(contents).hexdigest(),
            }
        ],
    }


def spdx_fragment(value: str) -> str:
    return "".join(ch if ch.isalnum() or ch in ".-" else "-" for ch in value)


if __name__ == "__main__":
    raise SystemExit(main())
