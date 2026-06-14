"""Contract tests for the backend-only capsem-builder CLI.

capsem-admin owns product profile validation, materialization, and image
builds. capsem-builder remains a backend helper for just/CI tasks only.
"""

from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import pytest
from click.testing import CliRunner

from capsem.builder.cli import cli


def test_help_exposes_only_backend_helper_commands() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["--help"])

    assert result.exit_code == 0
    lines = result.output.splitlines()
    start = lines.index("Commands:") + 1
    command_lines = [
        line.strip().split(maxsplit=1)[0]
        for line in lines[start:]
        if line.startswith("  ") and line.strip()
    ]
    assert set(command_lines) == {"doctor", "validate-skills", "agent", "audit"}
    assert "--dry-run" not in result.output


@pytest.mark.parametrize(
    "argv",
    [
        ["build"],
        ["build", "guest", "--dry-run"],
        ["validate"],
        ["inspect"],
        ["init"],
        ["new"],
        ["add"],
        ["mcp"],
    ],
)
def test_product_authoring_and_render_commands_are_removed(argv: list[str]) -> None:
    runner = CliRunner()
    result = runner.invoke(cli, argv)

    assert result.exit_code != 0
    assert "No such command" in result.output


def test_no_args_shows_backend_helper_help() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, [])

    assert result.exit_code == 0
    assert "doctor" in result.output
    assert "validate-skills" in result.output
    assert "\n  build" not in result.output


def test_version() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])

    assert result.exit_code == 0
    assert "capsem-builder" in result.output.lower()


def test_doctor_runs_profile_contract() -> None:
    from capsem.builder.doctor import CheckResult

    runner = CliRunner()
    with patch("capsem.builder.doctor.run_all_checks") as mock:
        mock.return_value = [
            CheckResult(name="profile-contract", passed=True, detail="profile code")
        ]
        result = runner.invoke(cli, ["doctor", "--profile", "code", "--config-root", "config"])

    assert result.exit_code == 0
    assert "capsem-builder doctor" in result.output
    assert "passed" in result.output
    mock.assert_called_once()
    assert mock.call_args.kwargs["profile_id"] == "code"


def test_doctor_rejects_positional_guest_dir() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["doctor", "guest/"])

    assert result.exit_code != 0
    assert "unexpected extra argument" in result.output.lower()


def test_agent_uses_profile_materialized_architecture(tmp_path: Path) -> None:
    guest = tmp_path / "materialized"
    guest.mkdir()
    arch = SimpleNamespace(rust_target="aarch64-unknown-linux-musl")
    config = SimpleNamespace(build=SimpleNamespace(architectures={"arm64": arch}))

    runner = CliRunner()
    with (
        patch("capsem.builder.cli.load_guest_config", return_value=config) as load_config,
        patch("capsem.builder.docker.cross_compile_agent") as cross_compile,
    ):
        result = runner.invoke(cli, ["agent", str(guest), "--arch", "arm64"])

    assert result.exit_code == 0
    load_config.assert_called_once_with(guest)
    cross_compile.assert_called_once()
    assert cross_compile.call_args.args[0] == "aarch64-unknown-linux-musl"


TRIVY_JSON_FIXTURE = json.dumps({
    "Results": [{
        "Target": "test",
        "Vulnerabilities": [
            {
                "VulnerabilityID": "CVE-2024-1234",
                "Severity": "HIGH",
                "PkgName": "openssl",
                "InstalledVersion": "3.0.13",
                "FixedVersion": "3.0.14",
            },
            {
                "VulnerabilityID": "CVE-2024-5678",
                "Severity": "LOW",
                "PkgName": "curl",
                "InstalledVersion": "7.88",
            },
        ],
    }],
})

TRIVY_NO_VULNS_FIXTURE = json.dumps({"Results": [{"Target": "test"}]})


def test_audit_from_file_reports_high_findings(tmp_path: Path) -> None:
    fixture = tmp_path / "trivy.json"
    fixture.write_text(TRIVY_JSON_FIXTURE)
    runner = CliRunner()

    result = runner.invoke(cli, ["audit", "--input", str(fixture)])

    assert result.exit_code == 1
    assert "CVE-2024-1234" in result.output
    assert "HIGH" in result.output


def test_audit_json_output_preserves_findings(tmp_path: Path) -> None:
    fixture = tmp_path / "trivy.json"
    fixture.write_text(TRIVY_JSON_FIXTURE)
    runner = CliRunner()

    result = runner.invoke(cli, ["audit", "--input", str(fixture), "--json"])

    assert result.exit_code == 1
    data = json.loads(result.output)
    assert len(data) == 2
    assert data[0]["id"] == "CVE-2024-1234"


def test_audit_no_vulns_exits_zero(tmp_path: Path) -> None:
    fixture = tmp_path / "trivy.json"
    fixture.write_text(TRIVY_NO_VULNS_FIXTURE)
    runner = CliRunner()

    result = runner.invoke(cli, ["audit", "--input", str(fixture)])

    assert result.exit_code == 0
    assert "Total: 0 vulnerabilities" in result.output


def test_audit_no_input_fails() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["audit"], input="")

    assert result.exit_code != 0
    assert "no input" in result.output

