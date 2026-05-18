from __future__ import annotations

from pathlib import Path
import textwrap

import pytest
from pydantic import ValidationError

from capsem.builder.profiles import (
    ManifestV3,
    ProfilePayloadV2,
    ProfileRevisionStatus,
    dump_manifest_json,
    dump_profile_json,
    validate_manifest_json,
    validate_profile_json,
    validate_profile_toml,
)


PROJECT_ROOT = Path(__file__).resolve().parents[1]
FIXTURE_DIR = PROJECT_ROOT / "schemas" / "fixtures"


def test_profile_payload_json_enters_and_leaves_through_pydantic() -> None:
    payload = (FIXTURE_DIR / "profile-v2-valid.json").read_text()

    profile = validate_profile_json(payload)
    dumped = dump_profile_json(profile)
    reparsed = ProfilePayloadV2.model_validate_json(dumped)

    assert profile == reparsed
    assert '"schema": "capsem.profile.v2"' in dumped
    assert '"@modelcontextprotocol/sdk"' in dumped


@pytest.mark.parametrize(
    "fixture_name",
    [
        "profile-v2-invalid-asset-hash.json",
        "profile-v2-invalid-extra-field.json",
        "profile-v2-invalid-tool-missing-version.json",
    ],
)
def test_profile_payload_rejects_invalid_golden_fixtures(fixture_name: str) -> None:
    payload = (FIXTURE_DIR / fixture_name).read_text()

    with pytest.raises(ValidationError):
        validate_profile_json(payload)


def test_profile_toml_immediately_validates_through_pydantic_json(tmp_path: Path) -> None:
    profile_path = tmp_path / "profile.toml"
    profile_path.write_text(
        textwrap.dedent(
            """
            schema = "capsem.profile.v2"
            version = 2
            id = "everyday-work"
            revision = "2026.0520.1"
            name = "Everyday Work"
            description = "Balanced defaults for day-to-day work."
            best_for = "Balanced defaults for day-to-day work."
            profile_type = "everyday-work"

            [compatibility]
            min_binary = "1.0.0"
            guest_abi = "capsem-guest-v2"

            [vm]
            memory_mib = 8192
            cpus = 4
            disk_mib = 32768
            network = "proxied"

            [vm.assets.arm64.kernel]
            url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/vmlinuz"
            hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/vmlinuz.minisig"
            size = 7797248
            content_type = "application/octet-stream"

            [vm.assets.arm64.initrd]
            url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/initrd.img"
            hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/initrd.img.minisig"
            size = 2270154
            content_type = "application/octet-stream"

            [vm.assets.arm64.rootfs]
            url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/rootfs.squashfs"
            hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/rootfs.squashfs.minisig"
            size = 454230016
            content_type = "application/vnd.squashfs"

            [packages.runtimes]
            python = "3.12.3"

            [packages.system]
            distro = "debian"
            release = "bookworm"

            [tools.capsem_doctor]
            version = "2026.05.18"
            required = true
            source = "guest"

            [security.capabilities]
            credential_brokerage = "ask"
            """
        )
    )

    profile = validate_profile_toml(profile_path)

    assert profile.id == "everyday-work"
    assert profile.vm.assets["arm64"].rootfs.hash.startswith("blake3:")


def test_manifest_status_enum_excludes_removed() -> None:
    assert [status.value for status in ProfileRevisionStatus] == [
        "active",
        "deprecated",
        "revoked",
    ]

    with pytest.raises(ValidationError):
        validate_manifest_json(
            """
            {
              "format": 3,
              "profiles": {
                "everyday-work": {
                  "current_revision": "2026.0520.1",
                  "revisions": {
                    "2026.0520.1": {
                      "status": "removed",
                      "min_binary": "1.0.0",
                      "profile_url": "https://assets.capsem.dev/profile.toml",
                      "profile_hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                      "profile_signature_url": "https://assets.capsem.dev/profile.toml.minisig"
                    }
                  }
                }
              }
            }
            """
        )


def test_manifest_current_revision_must_be_active() -> None:
    with pytest.raises(ValidationError):
        validate_manifest_json(
            """
            {
              "format": 3,
              "profiles": {
                "everyday-work": {
                  "current_revision": "2026.0520.1",
                  "revisions": {
                    "2026.0520.1": {
                      "status": "deprecated",
                      "min_binary": "1.0.0",
                      "profile_url": "https://assets.capsem.dev/profile.toml",
                      "profile_hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                      "profile_signature_url": "https://assets.capsem.dev/profile.toml.minisig"
                    }
                  }
                }
              }
            }
            """
        )


def test_manifest_json_round_trips_through_pydantic_dump() -> None:
    manifest = validate_manifest_json(
        """
        {
          "format": 3,
          "profiles": {
            "everyday-work": {
              "current_revision": "2026.0520.1",
              "revisions": {
                "2026.0520.1": {
                  "status": "active",
                  "min_binary": "1.0.0",
                  "max_binary": null,
                  "profile_url": "https://assets.capsem.dev/profile.toml",
                  "profile_hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                  "profile_signature_url": "https://assets.capsem.dev/profile.toml.minisig"
                }
              }
            }
          }
        }
        """
    )

    dumped = dump_manifest_json(manifest)
    reparsed = ManifestV3.model_validate_json(dumped)

    assert reparsed == manifest
    assert '"status": "active"' in dumped
