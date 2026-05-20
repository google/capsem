from __future__ import annotations

from pathlib import Path
import textwrap

import pytest
from pydantic import ValidationError

from capsem.builder.service_settings import (
    ServiceSettingsV2,
    create_service_settings_draft,
    dump_service_settings_json,
    dump_service_settings_schema_json,
    dump_service_settings_toml,
    validate_service_settings_json,
    validate_service_settings_toml,
)


PROJECT_ROOT = Path(__file__).resolve().parents[1]
FIXTURE_DIR = PROJECT_ROOT / "schemas" / "fixtures"
SCHEMA_PATH = PROJECT_ROOT / "schemas" / "capsem.service-settings.v2.schema.json"
DEFAULTS_FIXTURE_PATH = FIXTURE_DIR / "service-settings-v2-defaults.json"


def test_service_settings_minimal_json_enters_and_leaves_through_pydantic() -> None:
    settings = validate_service_settings_json(
        (FIXTURE_DIR / "service-settings-v2-minimal.json").read_text()
    )
    dumped = dump_service_settings_json(settings)
    reparsed = ServiceSettingsV2.model_validate_json(dumped)

    assert settings == reparsed
    assert settings.version == 1
    assert settings.profiles.default_profile == "everyday-work"
    assert settings.profile_catalog.check_interval_secs == 21600


def test_service_settings_defaults_match_committed_rust_contract(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CAPSEM_HOME", "/tmp/capsem-service-settings-defaults")
    settings = ServiceSettingsV2()

    assert dump_service_settings_json(settings) == DEFAULTS_FIXTURE_PATH.read_text(
        encoding="utf-8"
    ).rstrip("\n")
    assert settings.profiles.user_dirs == [
        "/tmp/capsem-service-settings-defaults/profiles"
    ]


def test_service_settings_complete_json_enters_and_leaves_through_pydantic() -> None:
    settings = validate_service_settings_json(
        (FIXTURE_DIR / "service-settings-v2-complete.json").read_text()
    )

    assert settings.app.appearance.theme.value == "dark"
    assert settings.assets.assets_dir == "/var/lib/capsem/assets"
    assert settings.credentials.items["openai.api_key"].value == "env:OPENAI_API_KEY"
    assert str(settings.profile_catalog.manifest_url) == (
        "https://profiles.example.com/capsem/manifest.json"
    )
    assert settings.corp_directives[0].operation.value == "lock"


def test_create_service_settings_draft_round_trips_through_json_and_toml(
    tmp_path: Path,
) -> None:
    draft = create_service_settings_draft(
        default_profile="corp-dev",
        base_dirs=["/opt/capsem/profiles/base"],
        corp_dirs=["/opt/capsem/profiles/corp"],
        user_dirs=["/var/lib/capsem/profiles/user"],
        assets_dir="/var/lib/capsem/assets",
    )
    json_payload = dump_service_settings_json(draft)
    reparsed_json = validate_service_settings_json(json_payload)

    assert reparsed_json.profiles.default_profile == "corp-dev"
    assert reparsed_json.profiles.base_dirs == ["/opt/capsem/profiles/base"]
    assert reparsed_json.profiles.corp_dirs == ["/opt/capsem/profiles/corp"]
    assert reparsed_json.assets.assets_dir == "/var/lib/capsem/assets"

    toml_path = tmp_path / "service.toml"
    toml_path.write_text(dump_service_settings_toml(draft), encoding="utf-8")
    reparsed_toml = validate_service_settings_toml(toml_path)

    assert reparsed_toml == reparsed_json


def test_service_settings_toml_json_toml_round_trip_is_canonical(
    tmp_path: Path,
) -> None:
    draft = create_service_settings_draft(
        default_profile="corp-dev",
        base_dirs=["/opt/capsem/profiles/base"],
        corp_dirs=["/opt/capsem/profiles/corp"],
        user_dirs=["/var/lib/capsem/profiles/user"],
        assets_dir="/var/lib/capsem/assets",
    )
    toml = dump_service_settings_toml(draft)

    settings_path = tmp_path / "service.toml"
    settings_path.write_text(toml, encoding="utf-8")
    from_toml = validate_service_settings_toml(settings_path)
    json_payload = dump_service_settings_json(from_toml)
    from_json = validate_service_settings_json(json_payload)
    toml2 = dump_service_settings_toml(from_json)

    assert toml == toml2


@pytest.mark.parametrize(
    "fixture_name",
    [
        "service-settings-v2-invalid-unknown-field.json",
        "service-settings-v2-invalid-profile-catalog.json",
        "service-settings-v2-invalid-profile-roots.json",
        "service-settings-v2-invalid-telemetry.json",
        "service-settings-v2-invalid-remote-policy.json",
        "service-settings-v2-invalid-credential.json",
        "service-settings-v2-invalid-assets.json",
    ],
)
def test_service_settings_rejects_invalid_golden_fixtures(fixture_name: str) -> None:
    with pytest.raises(ValidationError):
        validate_service_settings_json((FIXTURE_DIR / fixture_name).read_text())


def test_service_settings_toml_immediately_validates_through_pydantic_json(
    tmp_path: Path,
) -> None:
    settings_path = tmp_path / "service.toml"
    settings_path.write_text(
        textwrap.dedent(
            """
            version = 1

            [profiles]
            base_dirs = ["/Library/Application Support/Capsem/profiles/base"]
            corp_dirs = ["/Library/Application Support/Capsem/profiles/corp"]
            user_dirs = ["/Users/example/.capsem/profiles"]
            default_profile = "everyday-work"

            [profile_catalog]
            manifest_url = "https://profiles.example.com/capsem/manifest.json"
            profile_payload_pubkey = "RWQprofilepayloadpubkey"
            check_interval_secs = 300

            [telemetry]
            enabled = true
            endpoint = "https://otel.example.com/v1/traces"
            batch_max_events = 64
            flush_interval_ms = 1000

            [credentials.items."anthropic.api_key"]
            value = "env:ANTHROPIC_API_KEY"
            """
        ),
        encoding="utf-8",
    )

    settings = validate_service_settings_toml(settings_path)

    assert settings.telemetry.enabled is True
    assert settings.credentials.items["anthropic.api_key"].value == (
        "env:ANTHROPIC_API_KEY"
    )


def test_service_settings_schema_artifact_matches_pydantic_model() -> None:
    assert SCHEMA_PATH.read_text(encoding="utf-8") == dump_service_settings_schema_json()
