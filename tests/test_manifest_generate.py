from __future__ import annotations

from pathlib import Path

import blake3
from click.testing import CliRunner

from capsem.admin.cli import cli
from capsem.builder.manifest_generate import generate_profile_manifest
from capsem.builder.profiles import (
    ProfileManifest,
    create_profile_draft,
    dump_manifest_json,
    dump_profile_json,
    dump_profile_toml,
    validate_manifest_json,
)


def _blake3(payload: bytes) -> str:
    return f"blake3:{blake3.blake3(payload).hexdigest()}"


def test_generate_manifest_from_local_json_and_toml_profiles(tmp_path: Path) -> None:
    profiles_dir = tmp_path / "profiles"
    profiles_dir.mkdir()
    older = create_profile_draft("corp-dev", revision="2026.0520.2")
    newer = create_profile_draft("corp-dev", revision="2026.0520.10")
    older_path = profiles_dir / "corp-dev-2026.0520.2.json"
    newer_path = profiles_dir / "corp-dev-2026.0520.10.toml"
    older_bytes = dump_profile_json(older).encode()
    newer_bytes = dump_profile_toml(newer).encode()
    older_path.write_bytes(older_bytes)
    newer_path.write_bytes(newer_bytes)
    older_path.with_name(older_path.name + ".minisig").write_text(
        "signature\n",
        encoding="utf-8",
    )
    newer_path.with_name(newer_path.name + ".minisig").write_text(
        "signature\n",
        encoding="utf-8",
    )

    manifest = generate_profile_manifest(profiles_dir)
    dumped = dump_manifest_json(manifest)
    reparsed = ProfileManifest.model_validate_json(dumped)

    assert manifest == reparsed
    assert manifest.profiles["corp-dev"].current_revision == "2026.0520.10"
    older_record = manifest.profiles["corp-dev"].revisions["2026.0520.2"]
    newer_record = manifest.profiles["corp-dev"].revisions["2026.0520.10"]
    assert str(older_record.profile_url) == older_path.as_uri()
    assert str(newer_record.profile_url) == newer_path.as_uri()
    assert str(newer_record.profile_signature_url) == newer_path.as_uri() + ".minisig"
    assert older_record.profile_hash == _blake3(older_bytes)
    assert newer_record.profile_hash == _blake3(newer_bytes)


def test_generate_manifest_uses_base_url_and_status_overrides(tmp_path: Path) -> None:
    profiles_dir = tmp_path / "profiles"
    profiles_dir.mkdir()
    active = create_profile_draft("corp-dev", revision="2026.0520.3")
    deprecated = create_profile_draft("corp-dev", revision="2026.0520.2")
    active_path = profiles_dir / "corp-dev-2026.0520.3.json"
    deprecated_path = profiles_dir / "archive" / "corp-dev-2026.0520.2.json"
    deprecated_path.parent.mkdir()
    active_path.write_text(dump_profile_json(active), encoding="utf-8")
    deprecated_path.write_text(dump_profile_json(deprecated), encoding="utf-8")

    manifest = generate_profile_manifest(
        profiles_dir,
        base_url="https://profiles.example.invalid/catalog/",
        status_overrides={"corp-dev@2026.0520.2": "deprecated"},
    )

    revisions = manifest.profiles["corp-dev"].revisions
    assert manifest.profiles["corp-dev"].current_revision == "2026.0520.3"
    assert revisions["2026.0520.2"].status.value == "deprecated"
    assert str(revisions["2026.0520.2"].profile_url) == (
        "https://profiles.example.invalid/catalog/archive/corp-dev-2026.0520.2.json"
    )


def test_generate_manifest_rejects_duplicate_profile_revision(tmp_path: Path) -> None:
    profiles_dir = tmp_path / "profiles"
    profiles_dir.mkdir()
    profile = create_profile_draft("corp-dev", revision="2026.0520.3")
    (profiles_dir / "a.json").write_text(dump_profile_json(profile), encoding="utf-8")
    (profiles_dir / "b.toml").write_text(dump_profile_toml(profile), encoding="utf-8")

    try:
        generate_profile_manifest(profiles_dir)
    except ValueError as error:
        assert "duplicate profile revision" in str(error)
    else:
        raise AssertionError("duplicate profile revision should fail")


def test_capsem_admin_manifest_generate_writes_checkable_manifest(tmp_path: Path) -> None:
    profiles_dir = tmp_path / "profiles"
    profiles_dir.mkdir()
    profile = create_profile_draft("corp-dev", revision="2026.0520.3")
    profile_path = profiles_dir / "corp-dev.json"
    profile_path.write_text(dump_profile_json(profile), encoding="utf-8")
    profile_path.with_name(profile_path.name + ".minisig").write_text(
        "signature\n",
        encoding="utf-8",
    )
    output_path = tmp_path / "manifest.json"

    result = CliRunner().invoke(
        cli,
        [
            "manifest",
            "generate",
            "--profiles",
            str(profiles_dir),
            "--out",
            str(output_path),
        ],
    )

    assert result.exit_code == 0
    assert f"created {output_path}" in result.output
    manifest = validate_manifest_json(output_path.read_text(encoding="utf-8"))
    record = manifest.current_revision("corp-dev").record
    assert record.profile_hash == _blake3(profile_path.read_bytes())

    check_result = CliRunner().invoke(
        cli,
        ["manifest", "check", str(output_path), "--fast", "--json"],
    )
    assert check_result.exit_code == 0
    assert '"ok": true' in check_result.output
