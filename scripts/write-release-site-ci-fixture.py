#!/usr/bin/env python3
"""Write a deterministic release-channel fixture for the release-site CI job."""

from __future__ import annotations

import json
import sys
from pathlib import Path


ASSET_VERSION = "2030.0101.1"
BINARY_VERSION = "1.4.0"
DATE = "2030-01-01"

OBOM = (
    b'{"bomFormat":"CycloneDX","specVersion":"1.6","metadata":{"tools":'
    b'{"components":[{"name":"cdxgen","version":"11.0.0","type":"application"}]},'
    b'"component":{"name":"capsem-code-rootfs","type":"operating-system"}},'
    b'"components":[]}'
)

FILES: dict[str, dict[str, tuple[bytes, str]]] = {
    "arm64": {
        "vmlinuz": (
            b"kernel-arm64",
            "f9ae6e9bde83f2cbe5a22838340fdd2024c45a24516e5eb184248e1413aa41e4",
        ),
        "initrd.img": (
            b"initrd-arm64",
            "eb79698ac12564ac7dacc1dc6e3b55e8a93d655d1062d5c9a08ce22111c3cdb7",
        ),
        "rootfs.erofs": (
            b"rootfs-arm64",
            "5539d7bee1fcced4595ca2bcc327049fb87b3f4cf11323a1f65672bcca41604c",
        ),
        "obom.cdx.json": (
            OBOM,
            "759df3bd5cbe089be8a729b8c12a9d73ce7e6bf2874f6521ca60b5ed3e8af656",
        ),
    },
    "x86_64": {
        "vmlinuz": (
            b"kernel-x86_64",
            "1d89c0620e8b94a042d63647dec9337c9994233715927e57afac2ff7519de00f",
        ),
        "initrd.img": (
            b"initrd-x86_64",
            "72800ea9835c076eee979172f456b40581cd552dedb4ec48c7599077993c2139",
        ),
        "rootfs.erofs": (
            b"rootfs-x86_64",
            "3ace8945f4dac68744cb24bbbc638d727723e61173c5eec2b1500fd9463f50e4",
        ),
        "obom.cdx.json": (
            OBOM,
            "759df3bd5cbe089be8a729b8c12a9d73ce7e6bf2874f6521ca60b5ed3e8af656",
        ),
    },
}


def write_fixture(root: Path) -> None:
    assets_dir = root / "assets"
    arches: dict[str, dict[str, dict[str, object]]] = {}
    for arch, files in FILES.items():
        arch_dir = assets_dir / arch
        arch_dir.mkdir(parents=True, exist_ok=True)
        arches[arch] = {}
        for name, (contents, digest) in files.items():
            (arch_dir / name).write_bytes(contents)
            arches[arch][name] = {"hash": digest, "size": len(contents)}

    manifest = {
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": ASSET_VERSION,
            "releases": {
                ASSET_VERSION: {
                    "date": DATE,
                    "deprecated": False,
                    "min_binary": "1.0.0",
                    "arches": arches,
                }
            },
        },
        "binaries": {
            "current": BINARY_VERSION,
            "releases": {
                BINARY_VERSION: {
                    "date": DATE,
                    "deprecated": False,
                    "min_assets": ASSET_VERSION,
                    "files": [
                        {
                            "name": f"Capsem-{BINARY_VERSION}-macos-arm64.pkg",
                            "size": 123,
                            "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                        },
                        {
                            "name": "capsem-sbom.spdx.json",
                            "size": 456,
                            "sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                        },
                    ],
                }
            },
        },
    }
    (assets_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: write-release-site-ci-fixture.py <output-root>", file=sys.stderr)
        return 2
    write_fixture(Path(sys.argv[1]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
