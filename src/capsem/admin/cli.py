"""capsem-admin CLI.

Admin tooling validates public Capsem contracts through typed Pydantic models.
It does not manipulate raw settings dictionaries at command boundaries.
"""

from __future__ import annotations

from pathlib import Path
import tempfile
from typing import Literal

import click
from pydantic import BaseModel, ConfigDict, Field, ValidationError

from capsem.builder.config import load_guest_config
from capsem.builder.image_plan import (
    ImageArch,
    derive_image_plan,
    dump_image_plan_json,
)
from capsem.builder.image_verify import (
    ImageInventory,
    ImageInventoryMap,
    ImageVerificationArch,
    dump_image_verification_report_json,
    load_doctor_bundle_probe,
    load_image_inventory_json,
    verify_image_assets,
)
from capsem.builder.image_workspace import (
    ImageBuildReport,
    dump_image_build_report_json,
    dump_image_workspace_report_json,
    materialize_profile_image_workspace,
)
from capsem.builder.manifest_check import (
    check_profile_manifest_download,
    check_profile_manifest_fast,
    dump_manifest_check_report_json,
)
from capsem.builder.manifest_crypto import (
    dump_manifest_signature_verification_report_json,
    dump_manifest_sign_report_json,
    sign_manifest,
    verify_manifest_signature,
)
from capsem.builder.manifest_generate import generate_profile_manifest
from capsem.builder.profiles import (
    ProfilePayloadV2,
    ProfileType,
    ProfileUi,
    create_builtin_profile_drafts,
    create_profile_draft,
    dump_manifest_json,
    dump_profile_json,
    dump_profile_schema_json,
    dump_profile_toml,
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


def _load_image_inventories(
    plan_arches: list[ImageVerificationArch],
    assets_dir: Path,
    inventory_path: str | None,
) -> ImageInventoryMap:
    inventories: dict[ImageVerificationArch, tuple[Path | None, ImageInventory]] = {}
    if inventory_path is None:
        for arch in plan_arches:
            candidate = assets_dir / arch / "image-inventory.json"
            if candidate.exists():
                inventories[arch] = (candidate, load_image_inventory_json(candidate))
        return inventories

    root = Path(inventory_path)
    if root.is_dir():
        for arch in plan_arches:
            candidate = root / arch / "image-inventory.json"
            if not candidate.exists():
                raise ValueError(
                    f"missing image inventory for arch '{arch}': {candidate}"
                )
            inventories[arch] = (candidate, load_image_inventory_json(candidate))
        return inventories

    if len(plan_arches) != 1:
        raise ValueError(
            "--inventory FILE can only be used with a single --arch; "
            "pass an inventory directory for all-arch verification"
        )
    inventories[plan_arches[0]] = (root, load_image_inventory_json(root))
    return inventories


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


@cli.group()
def manifest() -> None:
    """Check and manage signed profile catalog manifests."""


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
    "--ui",
    "profile_ui",
    default=None,
    type=click.Choice([item.value for item in ProfileUi]),
    help="Frontend/workbench surface for this profile. Defaults from --profile-type.",
)
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
    profile_ui: str | None,
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
            ui=ProfileUi(profile_ui) if profile_ui is not None else None,
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


@profile.command("init-builtins")
@click.option("--revision", default=None, help="Profile revision for both generated profiles.")
@click.option(
    "--guest-dir",
    default="guest",
    show_default=True,
    type=click.Path(file_okay=False),
    help="Guest image config root to derive package/tool contracts from.",
)
@click.option(
    "--format",
    "output_format",
    type=click.Choice(["json", "toml"]),
    default="toml",
    show_default=True,
)
@click.option(
    "--out-dir",
    required=True,
    type=click.Path(file_okay=False),
    help="Directory that will receive everyday-work and coding profile files.",
)
@click.option("--force", is_flag=True, help="Overwrite generated files if they already exist.")
def profile_init_builtins(
    revision: str | None,
    guest_dir: str,
    output_format: str,
    out_dir: str,
    force: bool,
) -> None:
    """Generate the built-in Everyday and Coding Profile V2 payloads."""
    try:
        guest_config = load_guest_config(Path(guest_dir))
        drafts = create_builtin_profile_drafts(
            revision=revision,
            guest_config=guest_config,
        )
    except (FileNotFoundError, ValidationError) as error:
        raise click.ClickException(str(error)) from error

    output_dir = Path(out_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    suffix = "toml" if output_format == "toml" else "json"
    created: list[Path] = []
    for draft in drafts:
        path = output_dir / f"{draft.id}.profile.{suffix}"
        if path.exists() and not force:
            raise click.ClickException(f"{path} already exists; pass --force to overwrite")
        payload = (
            dump_profile_toml(draft)
            if output_format == "toml"
            else dump_profile_json(draft)
        )
        path.write_text(
            payload + ("" if payload.endswith("\n") else "\n"),
            encoding="utf-8",
        )
        created.append(path)

    click.echo(f"created {len(created)} profiles in {output_dir}")


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
    "--inventory",
    "inventory_path",
    default=None,
    type=click.Path(exists=True, file_okay=True, dir_okay=True),
    help=(
        "Optional inventory file for one arch, or directory containing "
        "<arch>/image-inventory.json. Defaults to auto-discover under --assets-dir."
    ),
)
@click.option(
    "--json",
    "json_output",
    is_flag=True,
    help="Emit a typed verification report.",
)
@click.option(
    "--doctor-bundle",
    "doctor_bundles",
    multiple=True,
    type=click.Path(exists=True, dir_okay=False),
    help="capsem-doctor --bundle tar from an in-VM boot probe.",
)
def image_verify(
    profile_path: str,
    assets_dir: str,
    arch: ImageArch,
    inventory_path: str | None,
    json_output: bool,
    doctor_bundles: tuple[str, ...],
) -> None:
    """Verify profile-declared image assets and optional image inventory."""
    try:
        profile = _load_profile(Path(profile_path))
        plan = derive_image_plan(profile, arch=arch)
        assets_root = Path(assets_dir)
        inventories = _load_image_inventories(
            [plan_arch.arch for plan_arch in plan.arches],
            assets_root,
            inventory_path,
        )
        report = verify_image_assets(
            plan,
            assets_root,
            inventories=inventories,
            probes=[
                load_doctor_bundle_probe(Path(bundle_path))
                for bundle_path in doctor_bundles
            ],
        )
    except (OSError, ValidationError, ValueError) as error:
        raise click.ClickException(str(error)) from error

    if json_output:
        click.echo(dump_image_verification_report_json(report))
    else:
        click.echo(f"profile: {report.profile_id}@{report.profile_revision}")
        click.echo(f"assets dir: {report.assets_dir}")
        ok_assets = sum(1 for asset in report.assets if asset.ok)
        click.echo(f"assets: {ok_assets}/{len(report.assets)} ok")
        if report.inventories:
            package_rows = [
                row
                for inventory in report.inventories
                for row in inventory.package_contract
            ]
            tool_rows = [
                row
                for inventory in report.inventories
                for row in inventory.tool_contract
            ]
            ok_packages = sum(1 for row in package_rows if row.ok)
            ok_tools = sum(1 for row in tool_rows if row.ok)
            click.echo(
                "package contract: "
                f"{ok_packages}/{len(package_rows)} ok"
            )
            click.echo(f"tool contract: {ok_tools}/{len(tool_rows)} ok")
        if report.probes:
            ok_probes = sum(1 for probe in report.probes if probe.ok)
            click.echo(f"probes: {ok_probes}/{len(report.probes)} ok")
        for asset in report.assets:
            if not asset.ok:
                click.echo(
                    f"{asset.arch}/{asset.kind}: {asset.failure} {asset.path}",
                    err=True,
                )
        for inventory in report.inventories:
            for row in [*inventory.package_contract, *inventory.tool_contract]:
                if not row.ok:
                    click.echo(
                        f"{inventory.arch}/{row.kind}/{row.name}: {row.failure} "
                        f"expected={row.expected_version} actual={row.actual_version}",
                        err=True,
                    )
        for probe in report.probes:
            if not probe.ok:
                click.echo(
                    f"{probe.kind}: failures={probe.failures} errors={probe.errors} "
                    f"tests={probe.tests} {probe.path}",
                    err=True,
                )

    if not report.ok:
        raise SystemExit(1)


@image.command("build-workspace")
@click.argument("profile_path", type=click.Path(exists=True, dir_okay=False))
@click.option(
    "--out",
    "out_dir",
    required=True,
    type=click.Path(file_okay=False),
    help="Directory where the profile-derived build workspace will be written.",
)
@click.option(
    "--arch",
    "arch",
    default="all",
    type=click.Choice(["all", "arm64", "x86_64"]),
    show_default=True,
)
@click.option("--force", is_flag=True, help="Write into a non-empty output directory.")
@click.option("--json", "json_output", is_flag=True, help="Emit a typed workspace report.")
def image_build_workspace(
    profile_path: str,
    out_dir: str,
    arch: ImageArch,
    force: bool,
    json_output: bool,
) -> None:
    """Materialize a build workspace from a Profile V2 package contract."""
    try:
        profile = _load_profile(Path(profile_path))
        report = materialize_profile_image_workspace(
            profile,
            Path(out_dir),
            arch=arch,
            force=force,
        )
    except (ValidationError, ValueError, FileExistsError) as error:
        raise click.ClickException(str(error)) from error

    if json_output:
        click.echo(dump_image_workspace_report_json(report))
        return

    click.echo(f"profile: {report.profile_id}@{report.profile_revision}")
    click.echo(f"workspace: {report.out_dir}")
    click.echo(f"package contract: {report.package_contract_hash}")
    click.echo("arches: " + ", ".join(report.arches))
    click.echo(f"files: {len(report.files)}")


@image.command("build")
@click.argument("profile_path", type=click.Path(exists=True, dir_okay=False))
@click.option(
    "--out",
    "output_dir",
    default="assets",
    show_default=True,
    type=click.Path(file_okay=False),
    help="Directory where built assets will be written.",
)
@click.option(
    "--workspace-dir",
    default=None,
    type=click.Path(file_okay=False),
    help="Optional directory for the generated profile-derived build workspace.",
)
@click.option(
    "--arch",
    "arch",
    default="all",
    type=click.Choice(["all", "arm64", "x86_64"]),
    show_default=True,
)
@click.option(
    "--template",
    default="rootfs",
    type=click.Choice(["rootfs", "kernel"]),
    show_default=True,
    help="Build one image template.",
)
@click.option(
    "--kernel-version",
    default=None,
    help="Explicit kernel version for kernel builds.",
)
@click.option("--dry-run", is_flag=True, help="Only materialize the profile-derived workspace.")
@click.option("--force", is_flag=True, help="Write into a non-empty workspace directory.")
@click.option("--json", "json_output", is_flag=True, help="Emit a typed build report.")
def image_build(
    profile_path: str,
    output_dir: str,
    workspace_dir: str | None,
    arch: ImageArch,
    template: Literal["rootfs", "kernel"],
    kernel_version: str | None,
    dry_run: bool,
    force: bool,
    json_output: bool,
) -> None:
    """Build profile-derived VM image assets."""
    from capsem.builder.config import load_guest_config
    from capsem.builder.docker import build_all_architectures, build_image

    temp_workspace: tempfile.TemporaryDirectory[str] | None = None
    try:
        profile = _load_profile(Path(profile_path))
        if workspace_dir is None:
            temp_workspace = tempfile.TemporaryDirectory(prefix="capsem-profile-image-")
            workspace_path = Path(temp_workspace.name)
            workspace_force = True
        else:
            workspace_path = Path(workspace_dir)
            workspace_force = force
        workspace_report = materialize_profile_image_workspace(
            profile,
            workspace_path,
            arch=arch,
            force=workspace_force,
        )
        config = load_guest_config(workspace_path)

        if not dry_run:
            out = Path(output_dir)
            if arch == "all":
                build_all_architectures(
                    config,
                    template=template,
                    output_dir=out,
                    kernel_version=kernel_version,
                    repo_root=Path.cwd(),
                )
            else:
                build_image(
                    config,
                    arch,
                    template=template,
                    output_dir=out,
                    kernel_version=kernel_version,
                    repo_root=Path.cwd(),
                )
        report = ImageBuildReport(
            ok=True,
            dry_run=dry_run,
            profile_id=workspace_report.profile_id,
            profile_revision=workspace_report.profile_revision,
            output_dir=str(Path(output_dir)),
            workspace=workspace_report,
            template=template,
        )
    except Exception as error:
        raise click.ClickException(str(error)) from error
    finally:
        if temp_workspace is not None:
            temp_workspace.cleanup()

    if json_output:
        click.echo(dump_image_build_report_json(report))
        return

    action = "planned" if dry_run else "built"
    click.echo(f"profile: {report.profile_id}@{report.profile_revision}")
    click.echo(f"{action}: {', '.join(report.workspace.arches)} {report.template}")
    click.echo(f"assets: {report.output_dir}")


@manifest.command("check")
@click.argument("manifest_path", type=click.Path(exists=True, dir_okay=False))
@click.option("--fast", "fast", is_flag=True, help="Run HEAD/local metadata checks.")
@click.option(
    "--download",
    "download",
    is_flag=True,
    help="Download and verify every referenced profile payload and VM asset.",
)
@click.option(
    "--download-dir",
    default=None,
    type=click.Path(file_okay=False),
    help="Directory for downloaded payloads and assets. Defaults to a temp directory.",
)
@click.option(
    "--pubkey",
    "pubkey_path",
    default=None,
    type=click.Path(exists=True, dir_okay=False),
    help="Minisign public key for cryptographic signature verification.",
)
@click.option("--json", "json_output", is_flag=True, help="Emit a typed check report.")
def manifest_check(
    manifest_path: str,
    fast: bool,
    download: bool,
    download_dir: str | None,
    pubkey_path: str | None,
    json_output: bool,
) -> None:
    """Check a Profile V2 catalog manifest."""
    if fast == download:
        raise click.ClickException("pass exactly one of --fast or --download")
    if fast and pubkey_path is not None:
        raise click.ClickException("--pubkey requires --download")

    try:
        report = (
            check_profile_manifest_download(
                Path(manifest_path),
                download_dir=Path(download_dir) if download_dir is not None else None,
                pubkey_path=Path(pubkey_path) if pubkey_path is not None else None,
            )
            if download
            else check_profile_manifest_fast(Path(manifest_path))
        )
    except (ValidationError, ValueError) as error:
        raise click.ClickException(str(error)) from error

    if json_output:
        click.echo(dump_manifest_check_report_json(report))
    else:
        checked = sum(len(profile.checks) for profile in report.profiles)
        failed = sum(
            1
            for profile in report.profiles
            for check in profile.checks
            if not check.ok
        )
        click.echo(f"manifest: {report.manifest_path}")
        click.echo(f"mode: {report.mode}")
        if report.download_dir is not None:
            click.echo(f"download dir: {report.download_dir}")
        click.echo(f"profiles: {len(report.profiles)}")
        click.echo(f"checks: {checked - failed}/{checked} ok")
        for profile in report.profiles:
            for check in profile.checks:
                if not check.ok:
                    click.echo(
                        (
                            f"{profile.profile_id}@{profile.revision} "
                            f"{check.kind}: {check.failure} {check.url}"
                        ),
                        err=True,
                    )

    if not report.ok:
        raise SystemExit(1)


def _parse_manifest_status_overrides(values: tuple[str, ...]) -> dict[str, str]:
    overrides: dict[str, str] = {}
    for value in values:
        if "=" not in value:
            raise click.ClickException("--status must use profile@revision=status")
        key, status = value.split("=", 1)
        if "@" not in key:
            raise click.ClickException("--status must use profile@revision=status")
        overrides[key] = status
    return overrides


def _parse_manifest_current_overrides(values: tuple[str, ...]) -> dict[str, str]:
    overrides: dict[str, str] = {}
    for value in values:
        if "=" not in value:
            raise click.ClickException("--current must use profile=revision")
        profile_id, revision = value.split("=", 1)
        overrides[profile_id] = revision
    return overrides


@manifest.command("generate")
@click.option(
    "--profiles",
    "profiles_dir",
    required=True,
    type=click.Path(exists=True, file_okay=False),
    help="Directory containing Profile V2 JSON/TOML payloads.",
)
@click.option(
    "--base-url",
    default=None,
    help="Base URL for generated profile payload URLs. Defaults to file:// URLs.",
)
@click.option(
    "--status",
    "status_overrides",
    multiple=True,
    help="Revision status override as profile@revision=active|deprecated|revoked.",
)
@click.option(
    "--current",
    "current_overrides",
    multiple=True,
    help="Current revision override as profile=revision.",
)
@click.option("--out", "output_path", default=None, type=click.Path(dir_okay=False))
@click.option("--force", is_flag=True, help="Overwrite --out if it already exists.")
def manifest_generate(
    profiles_dir: str,
    base_url: str | None,
    status_overrides: tuple[str, ...],
    current_overrides: tuple[str, ...],
    output_path: str | None,
    force: bool,
) -> None:
    """Generate a Profile V2 catalog manifest from local profile payloads."""
    try:
        manifest_payload = generate_profile_manifest(
            Path(profiles_dir),
            base_url=base_url,
            status_overrides=_parse_manifest_status_overrides(status_overrides),
            current_overrides=_parse_manifest_current_overrides(current_overrides),
        )
        payload = dump_manifest_json(manifest_payload)
    except (ValidationError, ValueError) as error:
        raise click.ClickException(str(error)) from error

    path = Path(output_path) if output_path is not None else None
    if path is None:
        click.echo(payload)
        return
    if path.exists() and not force:
        raise click.ClickException(f"{path} already exists; pass --force to overwrite")
    path.write_text(payload + ("" if payload.endswith("\n") else "\n"), encoding="utf-8")
    click.echo(f"created {path}")


@manifest.command("sign")
@click.argument("manifest_path", type=click.Path(exists=True, dir_okay=False))
@click.option(
    "--key",
    "key_path",
    required=True,
    type=click.Path(exists=True, dir_okay=False),
    help="Minisign secret key used to sign the manifest.",
)
@click.option(
    "--out",
    "signature_path",
    default=None,
    type=click.Path(dir_okay=False),
    help="Output signature path. Defaults to <manifest>.minisig.",
)
@click.option(
    "--password-file",
    default=None,
    type=click.Path(exists=True, dir_okay=False),
    help="Optional file containing the minisign key password.",
)
@click.option("--json", "json_output", is_flag=True, help="Emit a typed sign report.")
def manifest_sign(
    manifest_path: str,
    key_path: str,
    signature_path: str | None,
    password_file: str | None,
    json_output: bool,
) -> None:
    """Sign a Profile V2 catalog manifest with minisign."""
    try:
        report = sign_manifest(
            Path(manifest_path),
            Path(key_path),
            signature_path=Path(signature_path) if signature_path is not None else None,
            password_file=Path(password_file) if password_file is not None else None,
        )
    except RuntimeError as error:
        raise click.ClickException(str(error)) from error

    if json_output:
        click.echo(dump_manifest_sign_report_json(report))
    else:
        click.echo(f"signed {report.manifest_path}")
        click.echo(f"signature: {report.signature_path}")


@manifest.command("verify-signature")
@click.argument("manifest_path", type=click.Path(exists=True, dir_okay=False))
@click.option(
    "--signature",
    "signature_path",
    required=True,
    type=click.Path(exists=True, dir_okay=False),
    help="Minisign signature for the manifest.",
)
@click.option(
    "--pubkey",
    "pubkey_path",
    required=True,
    type=click.Path(exists=True, dir_okay=False),
    help="Minisign public key used to verify the manifest signature.",
)
@click.option("--json", "json_output", is_flag=True, help="Emit a typed verify report.")
def manifest_verify_signature(
    manifest_path: str,
    signature_path: str,
    pubkey_path: str,
    json_output: bool,
) -> None:
    """Verify a Profile V2 catalog manifest minisign signature."""
    report = verify_manifest_signature(
        Path(manifest_path),
        Path(signature_path),
        Path(pubkey_path),
    )
    if json_output:
        click.echo(dump_manifest_signature_verification_report_json(report))
    elif report.ok:
        click.echo(f"verified {report.manifest_path}")
    else:
        click.echo(report.message or "manifest signature verification failed", err=True)
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
