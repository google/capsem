from __future__ import annotations

from pathlib import Path

from click.testing import CliRunner
from unittest.mock import patch

from capsem.admin.cli import cli
from capsem.builder.doctor import CheckResult
from capsem.builder.profiles import (
    dump_profile_json,
    validate_profile_json,
    validate_profile_toml,
)
from capsem.builder.service_settings import (
    ServiceSettingsV2,
    dump_service_settings_json,
    validate_service_settings_json,
    validate_service_settings_toml,
)
from capsem.builder.security_packs import load_policy_context_fixture_jsonl


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


def test_capsem_admin_settings_init_json_matches_toml_reparsed_json(tmp_path) -> None:
    args = [
        "settings",
        "init",
        "--default-profile",
        "corp-dev",
        "--base-dir",
        "/opt/capsem/profiles/base",
        "--corp-dir",
        "/opt/capsem/profiles/corp",
        "--user-dir",
        "/var/lib/capsem/profiles/user",
        "--assets-dir",
        "/var/lib/capsem/assets",
    ]
    json_result = CliRunner().invoke(cli, args)
    toml_result = CliRunner().invoke(cli, [*args, "--format", "toml"])
    toml_path = tmp_path / "service.toml"
    toml_path.write_text(toml_result.output, encoding="utf-8")
    from_toml = validate_service_settings_toml(toml_path)

    assert json_result.exit_code == 0
    assert toml_result.exit_code == 0
    assert json_result.output.rstrip("\n") == dump_service_settings_json(from_toml)


def test_capsem_admin_settings_init_rejects_invalid_default_profile() -> None:
    result = CliRunner().invoke(cli, ["settings", "init", "--default-profile", "Bad"])

    assert result.exit_code == 1
    assert "default_profile" in result.output
    assert "pattern" in result.output


def _passing_admin_doctor_checks() -> list[CheckResult]:
    return [
        CheckResult(name="container-runtime", passed=True, detail="docker 27"),
        CheckResult(name="rust-toolchain", passed=True, detail="rustup 1.27"),
        CheckResult(name="b3sum", passed=True, detail="b3sum 1.5"),
        CheckResult(name="source-files", passed=True, detail="source files ok"),
    ]


def test_capsem_admin_doctor_json_uses_profile_not_guest_config() -> None:
    with patch(
        "capsem.admin.cli._admin_toolchain_checks",
        return_value=_passing_admin_doctor_checks(),
    ):
        result = CliRunner().invoke(
            cli,
            [
                "doctor",
                "--profile",
                "schemas/fixtures/profile-v2-valid.json",
                "--arch",
                "arm64",
                "--json",
            ],
        )

    assert result.exit_code == 0
    assert '"schema": "capsem.admin-doctor.v1"' in result.output
    assert '"profile_id": "everyday-work"' in result.output
    assert '"package_contract_hash": "blake3:' in result.output
    assert "guest-config" not in result.output
    assert "capsem-builder" not in result.output


def test_capsem_admin_doctor_fails_closed_on_profile_plan_error() -> None:
    with patch(
        "capsem.admin.cli._admin_toolchain_checks",
        return_value=_passing_admin_doctor_checks(),
    ):
        result = CliRunner().invoke(
            cli,
            [
                "doctor",
                "--profile",
                "schemas/fixtures/profile-v2-valid.json",
                "--json",
            ],
        )

    assert result.exit_code == 1
    assert '"ok": false' in result.output
    assert "missing VM assets for arch 'x86_64'" in result.output


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
            "--ui",
            "coding",
        ],
    )

    assert result.exit_code == 0
    profile = validate_profile_json(result.output)
    assert profile.id == "corp-dev"
    assert profile.revision == "2026.0520.7"
    assert profile.name == "Corp Dev"
    assert profile.ui.value == "coding"
    assert profile.editable.mcp_servers is True
    assert profile.editable.security_rules is True
    assert set(profile.vm.assets) == {"arm64", "x86_64"}


def test_capsem_admin_profile_init_everyday_type_defaults_everyday_ui() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "profile",
            "init",
            "everyday-work",
            "--revision",
            "2026.0520.7",
            "--profile-type",
            "everyday-work",
        ],
    )

    assert result.exit_code == 0
    profile = validate_profile_json(result.output)
    assert profile.profile_type.value == "everyday-work"
    assert profile.ui.value == "everyday"


def test_capsem_admin_profile_init_builtins_generates_typed_profiles(tmp_path) -> None:
    out_dir = tmp_path / "profiles"
    result = CliRunner().invoke(
        cli,
        [
            "profile",
            "init-builtins",
            "--revision",
            "2026.0520.10",
            "--out-dir",
            str(out_dir),
        ],
    )

    assert result.exit_code == 0, result.output
    everyday = validate_profile_toml(out_dir / "everyday-work.profile.toml")
    coding = validate_profile_toml(out_dir / "coding.profile.toml")
    assert everyday.id == "everyday-work"
    assert everyday.ui.value == "everyday"
    assert coding.id == "coding"
    assert coding.ui.value == "coding"
    assert everyday.packages == coding.packages
    assert coding.packages.system.apt["coreutils"] == "*"
    assert coding.packages.python_modules["pytest"] == "*"
    assert coding.packages.node_packages["@openai/codex"] == "*"
    assert coding.tools["codex"].required is True

    result = CliRunner().invoke(
        cli,
        ["profile", "init-builtins", "--out-dir", str(out_dir)],
    )

    assert result.exit_code == 1
    assert "already exists" in result.output


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


def test_capsem_admin_profile_init_writes_toml_and_refuses_overwrite(tmp_path) -> None:
    output_path = tmp_path / "corp-dev.profile.toml"
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
    profile = validate_profile_toml(output_path)
    assert profile.id == "corp-dev"
    assert profile.revision == "2026.0520.8"

    result = CliRunner().invoke(
        cli,
        ["profile", "init", "corp-dev", "--out", str(output_path)],
    )

    assert result.exit_code == 1
    assert "already exists" in result.output


def test_capsem_admin_profile_init_json_matches_toml_reparsed_json(tmp_path) -> None:
    args = [
        "profile",
        "init",
        "corp-dev",
        "--revision",
        "2026.0520.9",
        "--name",
        "Corp Dev",
    ]
    json_result = CliRunner().invoke(cli, args)
    toml_result = CliRunner().invoke(cli, [*args, "--format", "toml"])
    profile_path = tmp_path / "profile.toml"
    profile_path.write_text(toml_result.output, encoding="utf-8")
    from_toml = validate_profile_toml(profile_path)

    assert json_result.exit_code == 0
    assert toml_result.exit_code == 0
    assert json_result.output.rstrip("\n") == dump_profile_json(from_toml)


def test_capsem_admin_profile_init_rejects_invalid_profile_id() -> None:
    result = CliRunner().invoke(cli, ["profile", "init", "Bad"])

    assert result.exit_code == 1
    assert "id" in result.output
    assert "pattern" in result.output


def test_capsem_admin_image_plan_json_uses_profile_contract(tmp_path) -> None:
    profile_path = tmp_path / "corp-dev.profile.toml"
    init_result = CliRunner().invoke(
        cli,
        [
            "profile",
            "init",
            "corp-dev",
            "--revision",
            "2026.0520.10",
            "--out",
            str(profile_path),
        ],
    )
    assert init_result.exit_code == 0

    result = CliRunner().invoke(
        cli,
        ["image", "plan", str(profile_path), "--json"],
    )

    assert result.exit_code == 0
    assert '"schema": "capsem.image-plan.v1"' in result.output
    assert '"profile_id": "corp-dev"' in result.output
    assert '"arch": "arm64"' in result.output
    assert '"arch": "x86_64"' in result.output
    assert '"package_contract_hash": "blake3:' in result.output


def test_capsem_admin_image_plan_can_narrow_arch_for_partial_fixture() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "image",
            "plan",
            "schemas/fixtures/profile-v2-valid.json",
            "--arch",
            "arm64",
            "--json",
        ],
    )

    assert result.exit_code == 0
    assert '"profile_id": "everyday-work"' in result.output
    assert '"arch": "arm64"' in result.output
    assert '"arch": "x86_64"' not in result.output


def test_capsem_admin_image_plan_rejects_default_all_when_profile_lacks_arch() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "image",
            "plan",
            "schemas/fixtures/profile-v2-valid.json",
            "--json",
        ],
    )

    assert result.exit_code == 1
    assert "missing VM assets for arch 'x86_64'" in result.output


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
ui = "everyday"

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


def test_policy_context_corpus_loads_through_pydantic_models() -> None:
    fixtures = load_policy_context_fixture_jsonl(
        Path("data/policy-context/canonical-policy-contexts.jsonl")
    )

    assert [fixture.event_ref.event_id for fixture in fixtures] == [
        "evt-http-google-secret",
        "evt-http-github-clean",
    ]
    first = fixtures[0]
    assert first.event_ref.session_id == "session-s08c-corpus"
    assert first.context.common.profile_id == "coding"
    assert first.context.http.request is not None
    assert first.context.http.request.host == "googleapis.com"
    assert first.context.http.request.header("authorization").exists is True
    assert first.context.http.request.body.text == "token=secret"
    assert first.context.http.request.body.size == 12
    assert first.expected_labels == ["detect-google-secret"]


def test_capsem_admin_detection_backtest_uses_policy_context_corpus() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "detection",
            "backtest",
            "data/detection/sigma/google-secret-egress.yml",
            "--events",
            "data/policy-context/canonical-policy-contexts.jsonl",
            "--json",
        ],
    )

    assert result.exit_code == 0, result.output
    assert '"event_count": 2' in result.output
    assert '"match_count": 1' in result.output
    assert '"event_id": "evt-http-google-secret"' in result.output
    assert '"http.request.host": "googleapis.com"' in result.output
    assert '"http.request.body.text": "token=secret"' in result.output
