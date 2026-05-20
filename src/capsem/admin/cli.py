"""capsem-admin CLI.

Admin tooling validates public Capsem contracts through typed Pydantic models.
It does not manipulate raw settings dictionaries at command boundaries.
"""

from __future__ import annotations

from pathlib import Path
from typing import Literal

import click
from pydantic import BaseModel, ConfigDict, Field, ValidationError

from capsem.builder.profiles import (
    ProfilePayloadV2,
    dump_profile_schema_json,
    validate_profile_json,
    validate_profile_toml,
)
from capsem.builder.service_settings import (
    ServiceSettingsV2,
    dump_service_settings_schema_json,
    validate_service_settings_json,
    validate_service_settings_toml,
)


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, populate_by_name=True)


class AdminDiagnostic(StrictModel):
    path: str
    message: str


class SettingsValidationReport(StrictModel):
    schema_: Literal["capsem.service-settings.v2"] = Field(
        default="capsem.service-settings.v2",
        alias="schema",
    )
    ok: bool
    path: str
    diagnostics: list[AdminDiagnostic] = Field(default_factory=list)


class SettingsDoctorReport(SettingsValidationReport):
    default_profile: str | None = None
    profile_catalog_configured: bool | None = None
    telemetry_enabled: bool | None = None
    remote_policy_enabled: bool | None = None
    credential_backend: str | None = None


class ProfileValidationReport(StrictModel):
    schema_: Literal["capsem.profile.v2"] = Field(
        default="capsem.profile.v2",
        alias="schema",
    )
    ok: bool
    path: str
    diagnostics: list[AdminDiagnostic] = Field(default_factory=list)
    profile_id: str | None = None
    revision: str | None = None


def _diagnostics_from_error(error: Exception) -> list[AdminDiagnostic]:
    if isinstance(error, ValidationError):
        return [
            AdminDiagnostic(
                path=".".join(str(part) for part in item["loc"]),
                message=str(item["msg"]),
            )
            for item in error.errors()
        ]
    return [AdminDiagnostic(path="settings", message=str(error))]


def _load_settings(path: Path) -> ServiceSettingsV2:
    if path.suffix.lower() == ".json":
        return validate_service_settings_json(path.read_text(encoding="utf-8"))
    return validate_service_settings_toml(path)


def _load_profile(path: Path) -> ProfilePayloadV2:
    if path.suffix.lower() == ".json":
        return validate_profile_json(path.read_text(encoding="utf-8"))
    return validate_profile_toml(path)


def _validation_report(path: Path) -> SettingsValidationReport:
    try:
        _load_settings(path)
    except Exception as error:
        return SettingsValidationReport(
            ok=False,
            path=str(path),
            diagnostics=_diagnostics_from_error(error),
        )
    return SettingsValidationReport(ok=True, path=str(path))


def _profile_validation_report(path: Path) -> ProfileValidationReport:
    try:
        profile = _load_profile(path)
    except Exception as error:
        return ProfileValidationReport(
            ok=False,
            path=str(path),
            diagnostics=_diagnostics_from_error(error),
        )
    return ProfileValidationReport(
        ok=True,
        path=str(path),
        profile_id=profile.id,
        revision=profile.revision,
    )


def _doctor_report(path: Path) -> SettingsDoctorReport:
    try:
        settings = _load_settings(path)
    except Exception as error:
        return SettingsDoctorReport(
            ok=False,
            path=str(path),
            diagnostics=_diagnostics_from_error(error),
        )
    return SettingsDoctorReport(
        ok=True,
        path=str(path),
        default_profile=settings.profiles.default_profile,
        profile_catalog_configured=settings.profile_catalog.manifest_url is not None,
        telemetry_enabled=settings.telemetry.enabled,
        remote_policy_enabled=settings.remote_policy.enabled,
        credential_backend=settings.credentials.backend.value,
    )


@click.group(invoke_without_command=True)
@click.version_option(package_name="capsem", prog_name="capsem-admin")
@click.pass_context
def cli(ctx: click.Context) -> None:
    """Capsem admin tooling for corp profile and service-settings contracts."""
    if ctx.invoked_subcommand is None:
        click.echo(ctx.get_help())


@cli.group()
def settings() -> None:
    """Validate and inspect service settings."""


@cli.group()
def profile() -> None:
    """Validate and inspect Profile V2 payloads."""


@settings.command("schema")
def settings_schema() -> None:
    """Print the Service Settings V2 JSON Schema."""
    click.echo(dump_service_settings_schema_json())


@settings.command("validate")
@click.argument("settings_path", type=click.Path(exists=True, dir_okay=False))
@click.option("--json", "json_output", is_flag=True, help="Emit a typed JSON report.")
def settings_validate(settings_path: str, json_output: bool) -> None:
    """Validate service settings JSON or TOML."""
    path = Path(settings_path)
    report = _validation_report(path)
    if json_output:
        click.echo(report.model_dump_json(by_alias=True, indent=2))
    elif report.ok:
        click.echo("valid: service settings")
    else:
        for diagnostic in report.diagnostics:
            click.echo(f"{diagnostic.path}: {diagnostic.message}", err=True)
    if not report.ok:
        raise SystemExit(1)


@profile.command("schema")
def profile_schema() -> None:
    """Print the Profile V2 JSON Schema."""
    click.echo(dump_profile_schema_json())


@profile.command("validate")
@click.argument("profile_path", type=click.Path(exists=True, dir_okay=False))
@click.option("--json", "json_output", is_flag=True, help="Emit a typed JSON report.")
def profile_validate(profile_path: str, json_output: bool) -> None:
    """Validate a Profile V2 JSON or TOML payload."""
    path = Path(profile_path)
    report = _profile_validation_report(path)
    if json_output:
        click.echo(report.model_dump_json(by_alias=True, exclude_none=True, indent=2))
    elif report.ok:
        click.echo("valid: profile")
    else:
        for diagnostic in report.diagnostics:
            click.echo(f"{diagnostic.path}: {diagnostic.message}", err=True)
    if not report.ok:
        raise SystemExit(1)


@settings.command("doctor")
@click.argument("settings_path", type=click.Path(exists=True, dir_okay=False))
@click.option("--json", "json_output", is_flag=True, help="Emit a typed JSON report.")
def settings_doctor(settings_path: str, json_output: bool) -> None:
    """Validate service settings and summarize operational posture."""
    path = Path(settings_path)
    report = _doctor_report(path)
    if json_output:
        click.echo(report.model_dump_json(by_alias=True, indent=2))
    elif report.ok:
        click.echo("service settings: ok")
        click.echo(f"default profile: {report.default_profile}")
        click.echo(f"profile catalog configured: {report.profile_catalog_configured}")
        click.echo(f"telemetry enabled: {report.telemetry_enabled}")
        click.echo(f"remote policy enabled: {report.remote_policy_enabled}")
        click.echo(f"credential backend: {report.credential_backend}")
    else:
        for diagnostic in report.diagnostics:
            click.echo(f"{diagnostic.path}: {diagnostic.message}", err=True)
    if not report.ok:
        raise SystemExit(1)


def main() -> None:
    cli()
