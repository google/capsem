from __future__ import annotations

import json
from pathlib import Path
import textwrap

import blake3
import pytest
from pydantic import ValidationError

from capsem.builder.profiles import (
    ProfileManifest,
    ProfilePayloadV2,
    ProfileRevisionStatus,
    dump_manifest_json,
    dump_profile_json,
    validate_manifest_json,
    validate_profile_json,
    validate_profile_toml,
    verify_installable_profile_payload,
)


PROJECT_ROOT = Path(__file__).resolve().parents[1]
FIXTURE_DIR = PROJECT_ROOT / "schemas" / "fixtures"


def _profile_hash(payload: str) -> str:
    return f"blake3:{blake3.blake3(payload.encode()).hexdigest()}"


def _manifest_with_revision_hash(
    revision: str,
    status: str,
    profile_hash: str,
) -> str:
    return f"""
        {{
          "format": 1,
          "profiles": {{
            "everyday-work": {{
              "current_revision": "2026.0520.2",
              "revisions": {{
                "{revision}": {{
                  "status": "{status}",
                  "min_binary": "1.0.0",
                  "profile_url": "https://assets.capsem.dev/profile-{revision}.json",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "https://assets.capsem.dev/profile-{revision}.json.minisig"
                }},
                "2026.0520.2": {{
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "https://assets.capsem.dev/profile-2026.0520.2.json",
                  "profile_hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                  "profile_signature_url": "https://assets.capsem.dev/profile-2026.0520.2.json.minisig"
                }}
              }}
            }}
          }}
        }}
        """


def test_profile_payload_json_enters_and_leaves_through_pydantic() -> None:
    payload = (FIXTURE_DIR / "profile-v2-valid.json").read_text()

    profile = validate_profile_json(payload)
    dumped = dump_profile_json(profile)
    reparsed = ProfilePayloadV2.model_validate_json(dumped)

    assert profile == reparsed
    assert '"schema": "capsem.profile.v2"' in dumped
    assert profile.mcp_servers["github"].type_ == "stdio"
    assert profile.mcp_servers["github"].command == "npx"
    assert profile.mcp_servers["github"].capsem.allowed_tools == [
        "repo.read",
        "issue.write",
    ]
    assert str(profile.mcp_servers["corp-http"].url) == (
        "https://mcp.internal.example.com/mcp"
    )
    assert '"@modelcontextprotocol/sdk"' in dumped


def test_profile_payload_rejects_legacy_mcp_connectors_shape() -> None:
    payload = json.loads((FIXTURE_DIR / "profile-v2-valid.json").read_text())
    payload["mcp"] = {"connectors": payload.pop("mcpServers")}

    with pytest.raises(ValidationError):
        validate_profile_json(json.dumps(payload))


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
              "format": 1,
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


def test_manifest_status_lifecycle_gates_are_explicit() -> None:
    assert ProfileRevisionStatus.ACTIVE.can_be_current()
    assert ProfileRevisionStatus.ACTIVE.allows_install_or_update()
    assert ProfileRevisionStatus.ACTIVE.allows_new_vm()
    assert ProfileRevisionStatus.ACTIVE.allows_existing_vm()

    assert not ProfileRevisionStatus.DEPRECATED.can_be_current()
    assert not ProfileRevisionStatus.DEPRECATED.allows_install_or_update()
    assert not ProfileRevisionStatus.DEPRECATED.allows_new_vm()
    assert ProfileRevisionStatus.DEPRECATED.allows_existing_vm()

    assert not ProfileRevisionStatus.REVOKED.can_be_current()
    assert not ProfileRevisionStatus.REVOKED.allows_install_or_update()
    assert not ProfileRevisionStatus.REVOKED.allows_new_vm()
    assert not ProfileRevisionStatus.REVOKED.allows_existing_vm()


def test_manifest_current_revision_must_be_active() -> None:
    with pytest.raises(ValidationError):
        validate_manifest_json(
            """
            {
              "format": 1,
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


def test_manifest_resolves_current_and_specific_revision_records() -> None:
    manifest = validate_manifest_json(
        """
        {
          "format": 1,
          "profiles": {
            "everyday-work": {
              "current_revision": "2026.0520.2",
              "revisions": {
                "2026.0520.1": {
                  "status": "deprecated",
                  "min_binary": "1.0.0",
                  "profile_url": "https://assets.capsem.dev/profile-1.toml",
                  "profile_hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                  "profile_signature_url": "https://assets.capsem.dev/profile-1.toml.minisig"
                },
                "2026.0520.2": {
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "https://assets.capsem.dev/profile-2.toml",
                  "profile_hash": "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                  "profile_signature_url": "https://assets.capsem.dev/profile-2.toml.minisig"
                }
              }
            }
          }
        }
        """
    )

    current = manifest.current_revision("everyday-work")
    assert current.profile_id == "everyday-work"
    assert current.revision == "2026.0520.2"
    assert current.record.status is ProfileRevisionStatus.ACTIVE
    assert current.record.status.allows_install_or_update()

    deprecated = manifest.revision("everyday-work", "2026.0520.1")
    assert deprecated.profile_id == "everyday-work"
    assert deprecated.revision == "2026.0520.1"
    assert deprecated.record.status is ProfileRevisionStatus.DEPRECATED
    assert deprecated.record.status.allows_existing_vm()
    assert not deprecated.record.status.allows_new_vm()

    with pytest.raises(KeyError, match="profile 'ghost' not found"):
        manifest.current_revision("ghost")
    with pytest.raises(KeyError, match="revision '2026.0520.0'"):
        manifest.revision("everyday-work", "2026.0520.0")


def test_installable_profile_payload_verifies_manifest_hash_and_identity() -> None:
    payload = (FIXTURE_DIR / "profile-v2-valid.json").read_text()
    manifest = validate_manifest_json(
        _manifest_with_revision_hash("2026.0520.1", "active", _profile_hash(payload))
    )
    revision = manifest.revision("everyday-work", "2026.0520.1")

    verified = verify_installable_profile_payload(revision, payload)

    assert verified.profile.id == "everyday-work"
    assert verified.profile.revision == "2026.0520.1"
    assert verified.payload_hash == _profile_hash(payload)


def test_installable_profile_payload_rejects_status_hash_and_identity_drift() -> None:
    payload = (FIXTURE_DIR / "profile-v2-valid.json").read_text()
    deprecated_manifest = validate_manifest_json(
        _manifest_with_revision_hash("2026.0520.1", "deprecated", _profile_hash(payload))
    )
    with pytest.raises(ValueError, match="cannot be installed or updated"):
        verify_installable_profile_payload(
            deprecated_manifest.revision("everyday-work", "2026.0520.1"),
            payload,
        )

    mismatch_manifest = validate_manifest_json(
        _manifest_with_revision_hash(
            "2026.0520.1",
            "active",
            "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
    )
    with pytest.raises(ValueError, match="profile payload hash mismatch"):
        verify_installable_profile_payload(
            mismatch_manifest.revision("everyday-work", "2026.0520.1"),
            payload,
        )

    drifted_payload = payload.replace(
        '"revision": "2026.0520.1"',
        '"revision": "2026.0520.0"',
    )
    drift_manifest = validate_manifest_json(
        _manifest_with_revision_hash(
            "2026.0520.1",
            "active",
            _profile_hash(drifted_payload),
        )
    )
    with pytest.raises(ValueError, match="payload revision"):
        verify_installable_profile_payload(
            drift_manifest.revision("everyday-work", "2026.0520.1"),
            drifted_payload,
        )


def test_manifest_json_round_trips_through_pydantic_dump() -> None:
    manifest = validate_manifest_json(
        """
        {
          "format": 1,
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
    reparsed = ProfileManifest.model_validate_json(dumped)

    assert reparsed == manifest
    assert '"status": "active"' in dumped
