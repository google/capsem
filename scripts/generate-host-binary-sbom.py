#!/usr/bin/env python3
"""Generate a minimal SPDX SBOM for packaged Capsem host binaries."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
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
    with tempfile.TemporaryDirectory() as raw_tmp:
        raw = Path(raw_tmp)
        subprocess.run(["ar", "x", str(artifact.resolve())], cwd=raw, check=True)
        data = next(raw.glob("data.tar.*"), None)
        if data is None:
            raise SystemExit(f"{artifact} has no data.tar payload")
        payload = raw / "payload"
        payload.mkdir()
        subprocess.run(["tar", "xf", str(data.resolve()), "-C", str(payload)], check=True)
        return entries_from_payload(payload)


def pkg_entries(artifact: Path) -> list[dict[str, object]]:
    if shutil.which("pkgutil") is None:
        raise SystemExit("pkgutil is required to extract macOS .pkg artifacts")
    with tempfile.TemporaryDirectory() as raw_tmp:
        raw = Path(raw_tmp)
        expanded = raw / "expanded"
        subprocess.run(["pkgutil", "--expand-full", str(artifact.resolve()), str(expanded)], check=True)
        payloads = [path for path in expanded.rglob("Payload") if path.is_dir()]
        entries: list[dict[str, object]] = []
        for payload in payloads:
            entries.extend(entries_from_payload(payload))
        return entries


def entries_from_payload(payload: Path) -> list[dict[str, object]]:
    rows = []
    for path in payload.rglob("*"):
        if not path.is_file() or not os.access(path, os.X_OK):
            continue
        relative = path.relative_to(payload).as_posix().removeprefix("./")
        if not relative.startswith(EXECUTABLE_PREFIXES):
            continue
        contents = path.read_bytes()
        name = path.name
        rows.append(
            {
                "SPDXID": f"SPDXRef-File-{spdx_fragment(name)}",
                "fileName": f"/{relative}",
                "checksums": [
                    {
                        "algorithm": "SHA256",
                        "checksumValue": hashlib.sha256(contents).hexdigest(),
                    }
                ],
            }
        )
    return rows


def spdx_fragment(value: str) -> str:
    return "".join(ch if ch.isalnum() or ch in ".-" else "-" for ch in value)


if __name__ == "__main__":
    raise SystemExit(main())
