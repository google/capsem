from __future__ import annotations

from click.testing import CliRunner

from capsem.admin.cli import cli
from capsem.builder.service_settings import ServiceSettingsV2


def test_capsem_admin_settings_schema_outputs_service_settings_schema() -> None:
    result = CliRunner().invoke(cli, ["settings", "schema"])

    assert result.exit_code == 0
    assert '"title": "ServiceSettingsV2"' in result.output
    assert '"$defs"' in result.output


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
