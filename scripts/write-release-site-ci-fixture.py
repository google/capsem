#!/usr/bin/env python3
"""Write a deterministic release-channel fixture for the release-site CI job."""

from __future__ import annotations

import json
import argparse
import hashlib
from pathlib import Path

import blake3

ASSET_VERSION = "2030.0101.1"
BINARY_VERSION = "1.4.0"
DATE = "2030-01-01"
MAC_PACKAGE_FIXTURE_BLAKE3 = "448ff45531b52064b3bf401509c08ca3567bfbcde16aa54c6657a3cbb52d2766"
DEB_PACKAGE_FIXTURE_BLAKE3 = "ad071ff112d554af017c2eec1bdadfd153f6440813e8fded3075df5f5d22d4f0"
APP_BINARY_FIXTURE_BLAKE3 = "a2667ec38811444a55359d41a8c7d79e2ca9a03b941571e5c24afa49b0f7b08b"
TRAY_BINARY_FIXTURE_BLAKE3 = "7779eeb3aa3ef35dd7054359470ea6066ed309907ccba19e29236418c817f0f4"
SBOM_FIXTURE_BLAKE3 = "df2133a32b67cf97c9046915933d1449d886c245fedc97a6bf45078c25a19a2d"

OBOM = (
    b'{"bomFormat":"CycloneDX","specVersion":"1.6","metadata":{"tools":'
    b'{"components":[{"name":"cdxgen","version":"11.0.0","type":"application"}]},'
    b'"component":{"name":"capsem-code-rootfs","type":"operating-system"}},'
    b'"components":[]}'
)
SOFTWARE_INVENTORY = json.dumps(
    {
        "schema": "capsem.profile_software_inventory.v1",
        "architecture": "{arch}",
        "packages": [
            {
                "name": "python",
                "version": "3.12.11",
                "source": "dpkg",
                "architecture": "{arch}",
            },
            {
                "name": "@openai/codex",
                "version": "0.23.0",
                "source": "npm",
                "architecture": "all",
            },
        ],
    },
    indent=2,
    sort_keys=True,
).encode("utf-8")


def b3(payload: bytes) -> str:
    return blake3.blake3(payload).hexdigest()


SOFTWARE_INVENTORY_ARM64 = SOFTWARE_INVENTORY.replace(b"{arch}", b"arm64")
SOFTWARE_INVENTORY_X86_64 = SOFTWARE_INVENTORY.replace(b"{arch}", b"x86_64")

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
        "software-inventory.json": (
            SOFTWARE_INVENTORY_ARM64,
            b3(SOFTWARE_INVENTORY_ARM64),
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
        "software-inventory.json": (
            SOFTWARE_INVENTORY_X86_64,
            b3(SOFTWARE_INVENTORY_X86_64),
        ),
    },
}


def write_fixture(root: Path, *, include_binary_files: bool = True) -> None:
    assets_dir = root / "assets"
    arches: dict[str, dict[str, dict[str, object]]] = {}
    for arch, files in FILES.items():
        arch_dir = assets_dir / arch
        arch_dir.mkdir(parents=True, exist_ok=True)
        arches[arch] = {}
        for name, (contents, digest) in files.items():
            (arch_dir / name).write_bytes(contents)
            arches[arch][name] = {"hash": digest, "size": len(contents)}

    binary_release = {
        "date": DATE,
        "deprecated": False,
        "min_assets": ASSET_VERSION,
    }
    if include_binary_files:
        mac_package_payload = f"dry-run pkg for {BINARY_VERSION}\n".encode("utf-8")
        deb_package_payload = f"dry-run deb for {BINARY_VERSION}\n".encode("utf-8")
        binary_payload = f"dry-run capsem-app for {BINARY_VERSION}\n".encode("utf-8")
        tray_payload = f"dry-run capsem-tray for {BINARY_VERSION}\n".encode("utf-8")
        sbom_payload = json.dumps(
            {
                "spdxVersion": "SPDX-2.3",
                "name": "capsem-release-site-ci-fixture",
                "files": [
                    {
                        "SPDXID": "SPDXRef-File-capsem-app",
                        "fileName": "/usr/bin/capsem-app",
                        "checksums": [
                            {
                                "algorithm": "SHA256",
                                "checksumValue": hashlib.sha256(binary_payload).hexdigest(),
                            }
                        ],
                    },
                    {
                        "SPDXID": "SPDXRef-File-capsem-tray",
                        "fileName": "/usr/bin/capsem-tray",
                        "checksums": [
                            {
                                "algorithm": "SHA256",
                                "checksumValue": hashlib.sha256(tray_payload).hexdigest(),
                            }
                        ],
                    }
                ],
            },
            sort_keys=True,
        ).encode("utf-8")
        binary_release["files"] = [
            {
                "name": f"Capsem-{BINARY_VERSION}-macos-arm64.pkg",
                "size": len(mac_package_payload),
                "sha256": hashlib.sha256(mac_package_payload).hexdigest(),
                "blake3": MAC_PACKAGE_FIXTURE_BLAKE3,
                "binaries": [
                    {
                        "name": "capsem-app",
                        "installed_path": "/usr/bin/capsem-app",
                        "size": len(binary_payload),
                        "sha256": hashlib.sha256(binary_payload).hexdigest(),
                        "blake3": APP_BINARY_FIXTURE_BLAKE3,
                        "sbom_component_ref": "SPDXRef-File-capsem-app",
                    },
                    {
                        "name": "capsem-tray",
                        "installed_path": "/usr/bin/capsem-tray",
                        "size": len(tray_payload),
                        "sha256": hashlib.sha256(tray_payload).hexdigest(),
                        "blake3": TRAY_BINARY_FIXTURE_BLAKE3,
                        "sbom_component_ref": "SPDXRef-File-capsem-tray",
                    },
                ],
            },
            {
                "name": f"Capsem_{BINARY_VERSION}_arm64.deb",
                "size": len(deb_package_payload),
                "sha256": hashlib.sha256(deb_package_payload).hexdigest(),
                "blake3": DEB_PACKAGE_FIXTURE_BLAKE3,
                "binaries": [
                    {
                        "name": "capsem-app",
                        "installed_path": "/usr/bin/capsem-app",
                        "size": len(binary_payload),
                        "sha256": hashlib.sha256(binary_payload).hexdigest(),
                        "blake3": APP_BINARY_FIXTURE_BLAKE3,
                        "sbom_component_ref": "SPDXRef-File-capsem-app",
                    },
                    {
                        "name": "capsem-tray",
                        "installed_path": "/usr/bin/capsem-tray",
                        "size": len(tray_payload),
                        "sha256": hashlib.sha256(tray_payload).hexdigest(),
                        "blake3": TRAY_BINARY_FIXTURE_BLAKE3,
                        "sbom_component_ref": "SPDXRef-File-capsem-tray",
                    }
                ],
            },
            {
                "name": "capsem-sbom.spdx.json",
                "size": len(sbom_payload),
                "sha256": hashlib.sha256(sbom_payload).hexdigest(),
                "blake3": SBOM_FIXTURE_BLAKE3,
            },
        ]

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
                BINARY_VERSION: binary_release
            },
        },
    }
    (assets_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output_root")
    parser.add_argument(
        "--without-binary-files",
        action="store_true",
        help="Omit host binary package/SBOM file metadata for bootstrap channel validation.",
    )
    args = parser.parse_args()
    write_fixture(Path(args.output_root), include_binary_files=not args.without_binary_files)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
