"""capsem-admin CLI.

Admin tooling validates public Capsem contracts through typed Pydantic models.
It does not manipulate raw settings dictionaries at command boundaries.
"""

from __future__ import annotations

from pathlib import Path
from typing import Literal

import click
from pydantic import BaseModel, ConfigDict, Field, ValidationError

from capsem.builder.image_plan import (
    ImageArch,
    derive_image_plan,
    dump_image_plan_json,
)
from capsem.builder.image_verify import (
    dump_image_verification_report_json,
    verify_image_assets,
)
from capsem.builder.profiles import (
    ProfileType,
    create_profile_draft,
    dump_profile_json,
    dump_profile_toml,
    ProfilePayloadV2,
    dump_profile_schema_json,
    validate_profile_json,
    validate_profile_toml,
)
from capsem.builder.service_settings import (
    ServiceSettingsV2,
    create_service_settings_draft,
    dump_service_settings_schema_json,
    dump_service_settings_json,
    dump_service_settings_toml,
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


@cli.group()
def image() -> None:
    """Derive and verify profile-backed image build plans."""


@settings.command("schema")
def settings_schema() -> None:
    """Print the Service Settings V2 JSON Schema."""
    click.echo(dump_service_settings_schema_json())


@settings.command("init")
@click.option("--default-profile", default="everyday-work", show_default=True)
@click.option(
    "--base-dir",
    "base_dirs",
    multiple=True,
    help="Base profile directory. Repeat for multiple directories.",
)
@click.option(
    "--corp-dir",
    "corp_dirs",
    multiple=True,
    help="Corp profile directory. Repeat for multiple directories.",
)
@click.option(
    "--user-dir",
    "user_dirs",
    multiple=True,
    help="User profile directory. Repeat for multiple directories.",
)
@click.option("--assets-dir", default=None, help="Local profile VM asset cache directory.")
@click.option(
    "--format",
    "output_format",
    type=click.Choice(["json", "toml"]),
    default=None,
    help="Output format. Defaults to --out suffix, or json for stdout.",
)
@click.option("--out", "output_path", default=None, type=click.Path(dir_okay=False))
@click.option("--force", is_flag=True, help="Overwrite --out if it already exists.")
def settings_init(
    default_profile: str,
    base_dirs: tuple[str, ...],
    corp_dirs: tuple[str, ...],
    user_dirs: tuple[str, ...],
    assets_dir: str | None,
    output_format: str | None,
    output_path: str | None,
    force: bool,
) -> None:
    """Create a valid Service Settings V2 draft."""
    try:
        draft = create_service_settings_draft(
            default_profile=default_profile,
            base_dirs=list(base_dirs) or None,
            corp_dirs=list(corp_dirs) or None,
            user_dirs=list(user_dirs) or None,
            assets_dir=assets_dir,
        )
    except ValidationError as error:
        raise click.ClickException(str(error)) from error

    path = Path(output_path) if output_path is not None else None
    resolved_format = output_format
    if resolved_format is None:
        resolved_format = (
            "toml" if path is not None and path.suffix.lower() == ".toml" else "json"
        )
    payload = (
        dump_service_settings_toml(draft)
        if resolved_format == "toml"
        else dump_service_settings_json(draft)
    )

    if path is None:
        click.echo(payload, nl=not payload.endswith("\n"))
        return
    if path.exists() and not force:
        raise click.ClickException(f"{path} already exists; pass --force to overwrite")
    path.write_text(payload + ("" if payload.endswith("\n") else "\n"), encoding="utf-8")
    click.echo(f"created {path}")


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


@profile.command("init")
@click.argument("profile_id")
@click.option("--revision", default=None, help="Profile revision, for example 2026.0520.1.")
@click.option("--name", default=None, help="Human-readable profile name.")
@click.option("--description", default=None, help="Human-readable profile description.")
@click.option("--best-for", default=None, help="Short operator-facing profile fit summary.")
@click.option(
    "--profile-type",
    default=ProfileType.CODING.value,
    type=click.Choice([item.value for item in ProfileType]),
    show_default=True,
)
@click.option(
    "--format",
    "output_format",
    type=click.Choice(["json", "toml"]),
    default=None,
    help="Output format. Defaults to --out suffix, or json for stdout.",
)
@click.option("--out", "output_path", default=None, type=click.Path(dir_okay=False))
@click.option("--force", is_flag=True, help="Overwrite --out if it already exists.")
def profile_init(
    profile_id: str,
    revision: str | None,
    name: str | None,
    description: str | None,
    best_for: str | None,
    profile_type: str,
    output_format: str | None,
    output_path: str | None,
    force: bool,
) -> None:
    """Create a valid Profile V2 JSON draft."""
    try:
        draft = create_profile_draft(
            profile_id,
            revision=revision,
            name=name,
            description=description,
            best_for=best_for,
            profile_type=ProfileType(profile_type),
        )
    except ValidationError as error:
        raise click.ClickException(str(error)) from error

    path = Path(output_path) if output_path is not None else None
    resolved_format = output_format
    if resolved_format is None:
        resolved_format = (
            "toml" if path is not None and path.suffix.lower() == ".toml" else "json"
        )
    payload = (
        dump_profile_toml(draft)
        if resolved_format == "toml"
        else dump_profile_json(draft)
    )

    if path is None:
        click.echo(payload, nl=not payload.endswith("\n"))
        return

    if path.exists() and not force:
        raise click.ClickException(f"{path} already exists; pass --force to overwrite")
    path.write_text(payload + ("" if payload.endswith("\n") else "\n"), encoding="utf-8")
    click.echo(f"created {path}")


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


@image.command("plan")
@click.argument("profile_path", type=click.Path(exists=True, dir_okay=False))
@click.option(
    "--arch",
    "arch",
    default="all",
    type=click.Choice(["all", "arm64", "x86_64"]),
    show_default=True,
)
@click.option("--json", "json_output", is_flag=True, help="Emit the typed image plan.")
def image_plan(profile_path: str, arch: ImageArch, json_output: bool) -> None:
    """Derive an image build plan from a Profile V2 payload."""
    try:
        profile = _load_profile(Path(profile_path))
        plan = derive_image_plan(profile, arch=arch)
    except (ValidationError, ValueError) as error:
        raise click.ClickException(str(error)) from error

    if json_output:
        click.echo(dump_image_plan_json(plan))
        return

    click.echo(f"profile: {plan.profile_id}@{plan.profile_revision}")
    click.echo(f"guest ABI: {plan.guest_abi}")
    click.echo(f"package contract: {plan.package_contract_hash}")
    click.echo("arches: " + ", ".join(item.arch for item in plan.arches))
    click.echo(f"system: {plan.packages.system.distro} {plan.packages.system.release}")
    click.echo(f"tools: {len(plan.tools)}")


@image.command("verify")
@click.argument("profile_path", type=click.Path(exists=True, dir_okay=False))
@click.option(
    "--assets-dir",
    required=True,
    type=click.Path(exists=True, file_okay=False),
    help="Local asset directory laid out as <assets-dir>/<arch>/<asset filename>.",
)
@click.option(
    "--arch",
    "arch",
    default="all",
    type=click.Choice(["all", "arm64", "x86_64"]),
    show_default=True,
)
@click.option(
    "--json",
    "json_output",
    is_flag=True,
    help="Emit a typed verification report.",
)
def image_verify(
    profile_path: str,
    assets_dir: str,
    arch: ImageArch,
    json_output: bool,
) -> None:
    """Verify profile-declared image assets in a local asset directory."""
    try:
        profile = _load_profile(Path(profile_path))
        plan = derive_image_plan(profile, arch=arch)
        report = verify_image_assets(plan, Path(assets_dir))
    except (ValidationError, ValueError) as error:
        raise click.ClickException(str(error)) from error

    if json_output:
        click.echo(dump_image_verification_report_json(report))
    else:
        click.echo(f"profile: {report.profile_id}@{report.profile_revision}")
        click.echo(f"assets dir: {report.assets_dir}")
        ok_assets = sum(1 for asset in report.assets if asset.ok)
        click.echo(f"assets: {ok_assets}/{len(report.assets)} ok")
        for asset in report.assets:
            if not asset.ok:
                click.echo(
                    f"{asset.arch}/{asset.kind}: {asset.failure} {asset.path}",
                    err=True,
                )

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
