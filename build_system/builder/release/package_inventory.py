"""Package inventory from an exact flattened image, without executing its code."""
from __future__ import annotations

import json
import re
import tarfile
from pathlib import Path, PurePosixPath
from typing import Any

from ..crypto import sha256


def _read(image: tarfile.TarFile, path: str, limit: int) -> bytes:
    matches = [entry for entry in image.getmembers()
               if PurePosixPath(entry.name) == PurePosixPath(path.lstrip("/"))]
    if len(matches) != 1 or not matches[0].isfile():
        raise ValueError(f"missing or ambiguous regular inventory file: {path}")
    if matches[0].size > limit:
        raise ValueError(f"inventory file too large: {path}")
    stream = image.extractfile(matches[0])
    assert stream is not None
    return stream.read()


def _packages(status: str) -> list[dict[str, str]]:
    packages = []
    for paragraph in re.split(r"\n\s*\n", status):
        fields = dict(line.split(": ", 1) for line in paragraph.splitlines()
                      if not line.startswith((" ", "\t")) and ": " in line)
        if fields.get("Status") != "install ok installed":
            continue
        if not all(fields.get(key) for key in ("Package", "Version", "Architecture")):
            raise ValueError("incomplete installed package record")
        packages.append({"name": fields["Package"], "version": fields["Version"],
                         "architecture": fields["Architecture"]})
    if not packages:
        raise ValueError("missing installed packages")
    return sorted(packages, key=lambda entry: (entry["name"], entry["architecture"]))


def from_rootfs(source: Path, subject: str, architecture: str) -> dict[str, Any]:
    if not re.fullmatch(r"[^@\s]+@sha256:[0-9a-f]{64}", subject):
        raise ValueError("inventory subject must be an exact image digest")
    if architecture not in {"arm64", "amd64"}:
        raise ValueError("unsupported inventory architecture")
    with tarfile.open(source) as image:
        packages = _packages(_read(image, "var/lib/dpkg/status", 32 * 1024 * 1024).decode())
        toolkit = json.loads(_read(image, "usr/local/share/capsem/toolkit-inventory.json", 1024 * 1024))
        if toolkit["architecture"] != architecture:
            raise ValueError("toolkit architecture differs from image")
        for tool in toolkit["tools"]:
            binary = _read(image, tool["binary_path"], 256 * 1024 * 1024)
            if sha256(binary).hexdigest() != tool["binary_sha256"]:
                raise ValueError(f"{tool['name']}: inventoried binary changed in descendant image")
    return {"schema": "capsem.package_inventory.v1", "subject": subject,
            "architecture": architecture, "packages": packages, "tools": toolkit["tools"]}
