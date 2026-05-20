from __future__ import annotations

from click.testing import CliRunner

from capsem.admin.cli import cli
from capsem.builder.profiles import validate_profile_json
from capsem.builder.service_settings import (
    ServiceSettingsV2,
    validate_service_settings_json,
    validate_service_settings_toml,
)


def test_capsem_admin_settings_schema_outputs_service_settings_schema() -> None:
    result = CliRunner().invoke(cli, ["settings", "schema"])

    assert result.exit_code == 0
    assert '"title": "ServiceSettingsV2"' in result.output
    assert '"$defs"' in result.output


def test_capsem_admin_settings_init_outputs_valid_json_settings() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "settings",
            "init",
            "--default-profile",
            "corp-dev",
            "--base-dir",
            "/opt/capsem/profiles/base",
            "--user-dir",
            "/var/lib/capsem/profiles/user",
        ],
    )

    assert result.exit_code == 0
    settings = validate_service_settings_json(result.output)
    assert settings.profiles.default_profile == "corp-dev"
    assert settings.profiles.base_dirs == ["/opt/capsem/profiles/base"]
    assert settings.profiles.user_dirs == ["/var/lib/capsem/profiles/user"]


def test_capsem_admin_settings_init_writes_toml_and_refuses_overwrite(tmp_path) -> None:
    output_path = tmp_path / "service.toml"
    result = CliRunner().invoke(
        cli,
        [
            "settings",
            "init",
            "--default-profile",
            "corp-dev",
            "--corp-dir",
            "/opt/capsem/profiles/corp",
            "--out",
            str(output_path),
        ],
    )

    assert result.exit_code == 0
    assert f"created {output_path}" in result.output
    settings = validate_service_settings_toml(output_path)
    assert settings.profiles.default_profile == "corp-dev"
    assert settings.profiles.corp_dirs == ["/opt/capsem/profiles/corp"]

    result = CliRunner().invoke(cli, ["settings", "init", "--out", str(output_path)])

    assert result.exit_code == 1
    assert "already exists" in result.output


def test_capsem_admin_settings_init_rejects_invalid_default_profile() -> None:
    result = CliRunner().invoke(cli, ["settings", "init", "--default-profile", "Bad"])

    assert result.exit_code == 1
    assert "default_profile" in result.output
    assert "pattern" in result.output


def test_capsem_admin_profile_schema_outputs_profile_schema() -> None:
    result = CliRunner().invoke(cli, ["profile", "schema"])

    assert result.exit_code == 0
    assert '"title": "ProfilePayloadV2"' in result.output
    assert '"capsem.profile.v2"' in result.output


def test_capsem_admin_profile_init_outputs_valid_json_profile() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "profile",
            "init",
            "corp-dev",
            "--revision",
            "2026.0520.7",
            "--name",
            "Corp Dev",
        ],
    )

    assert result.exit_code == 0
    profile = validate_profile_json(result.output)
    assert profile.id == "corp-dev"
    assert profile.revision == "2026.0520.7"
    assert profile.name == "Corp Dev"
    assert profile.editable.mcp_servers is True
    assert profile.editable.security_rules is True
    assert set(profile.vm.assets) == {"arm64", "x86_64"}


def test_capsem_admin_profile_init_writes_file_and_refuses_overwrite(tmp_path) -> None:
    output_path = tmp_path / "corp-dev.profile.json"
    result = CliRunner().invoke(
        cli,
        [
            "profile",
            "init",
            "corp-dev",
            "--revision",
            "2026.0520.8",
            "--out",
            str(output_path),
        ],
    )

    assert result.exit_code == 0
    assert f"created {output_path}" in result.output
    profile = validate_profile_json(output_path.read_text(encoding="utf-8"))
    assert profile.id == "corp-dev"
    assert profile.revision == "2026.0520.8"

    result = CliRunner().invoke(
        cli,
        ["profile", "init", "corp-dev", "--out", str(output_path)],
    )

    assert result.exit_code == 1
    assert "already exists" in result.output


def test_capsem_admin_profile_init_rejects_invalid_profile_id() -> None:
    result = CliRunner().invoke(cli, ["profile", "init", "Bad"])

    assert result.exit_code == 1
    assert "id" in result.output
    assert "pattern" in result.output


def test_capsem_admin_settings_validate_accepts_json_fixture() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "settings",
            "validate",
            "schemas/fixtures/service-settings-v2-complete.json",
        ],
    )

    assert result.exit_code == 0
    assert "valid: service settings" in result.output


def test_capsem_admin_profile_validate_accepts_json_fixture() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "profile",
            "validate",
            "schemas/fixtures/profile-v2-valid.json",
        ],
    )

    assert result.exit_code == 0
    assert "valid: profile" in result.output


def test_capsem_admin_settings_validate_json_report_round_trips_through_pydantic() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "settings",
            "validate",
            "schemas/fixtures/service-settings-v2-minimal.json",
            "--json",
        ],
    )

    assert result.exit_code == 0
    assert '"schema": "capsem.service-settings.v2"' in result.output
    assert '"ok": true' in result.output


def test_capsem_admin_profile_validate_json_report_round_trips_through_pydantic() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "profile",
            "validate",
            "schemas/fixtures/profile-v2-valid.json",
            "--json",
        ],
    )

    assert result.exit_code == 0
    assert '"schema": "capsem.profile.v2"' in result.output
    assert '"ok": true' in result.output
    assert '"profile_id": "everyday-work"' in result.output
    assert '"revision": "2026.0520.1"' in result.output


def test_capsem_admin_settings_validate_rejects_invalid_json_fixture() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "settings",
            "validate",
            "schemas/fixtures/service-settings-v2-invalid-telemetry.json",
        ],
    )

    assert result.exit_code == 1
    assert "telemetry" in result.output
    assert "endpoint" in result.output


def test_capsem_admin_profile_validate_rejects_invalid_json_fixture() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "profile",
            "validate",
            "schemas/fixtures/profile-v2-invalid-extra-field.json",
        ],
    )

    assert result.exit_code == 1
    assert "extra" in result.output.lower()


def test_capsem_admin_settings_doctor_json_summarizes_posture() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "settings",
            "doctor",
            "schemas/fixtures/service-settings-v2-complete.json",
            "--json",
        ],
    )

    assert result.exit_code == 0
    assert '"profile_catalog_configured": true' in result.output
    assert '"telemetry_enabled": true' in result.output
    assert '"remote_policy_enabled": true' in result.output


def test_capsem_admin_settings_doctor_accepts_toml(tmp_path) -> None:
    settings_path = tmp_path / "service.toml"
    settings = ServiceSettingsV2()
    settings_path.write_text(
        """
version = 1

[profiles]
base_dirs = ["/Library/Application Support/Capsem/profiles/base"]
user_dirs = ["/Users/example/.capsem/profiles"]
default_profile = "everyday-work"
""".lstrip(),
        encoding="utf-8",
    )

    result = CliRunner().invoke(
        cli,
        ["settings", "doctor", str(settings_path)],
    )

    assert settings.profiles.default_profile == "everyday-work"
    assert result.exit_code == 0
    assert "service settings: ok" in result.output
    assert "default profile: everyday-work" in result.output


def test_capsem_admin_profile_validate_accepts_toml(tmp_path) -> None:
    profile_path = tmp_path / "profile.toml"
    profile_path.write_text(
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
""".lstrip(),
        encoding="utf-8",
    )

    result = CliRunner().invoke(
        cli,
        ["profile", "validate", str(profile_path), "--json"],
    )

    assert result.exit_code == 0
    assert '"profile_id": "everyday-work"' in result.output
