"""Capsem builder CLI -- config-driven guest image tooling.

Commands:
  validate  Lint and validate guest config
  build     Render Dockerfiles (--dry-run) or build images
  inspect   Show config summary
  init      Scaffold a new guest config directory
  add       Add AI provider, package set, or MCP server templates
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import click

from capsem.builder.config import generate_defaults_json, load_guest_config
from capsem.builder.docker import render_dockerfile
from capsem.builder.scaffold import (
    add_ai_provider,
    add_mcp_server,
    add_package_set,
    init_guest_dir,
)
from capsem.builder.validate import Severity, validate_guest


@click.group(invoke_without_command=True)
@click.version_option(package_name="capsem", prog_name="capsem-builder")
@click.pass_context
def cli(ctx: click.Context) -> None:
    """Capsem builder -- config-driven guest image tooling."""
    if ctx.invoked_subcommand is None:
        click.echo(ctx.get_help())


# ---------------------------------------------------------------------------
# validate
# ---------------------------------------------------------------------------


@cli.command()
@click.argument("guest_dir", default="guest", type=click.Path(exists=False))
@click.option("--artifacts", type=click.Path(exists=True), default=None,
              help="Artifacts directory to check (capsem-init, CA cert, etc.)")
def validate(guest_dir: str, artifacts: str | None) -> None:
    """Validate a guest image configuration."""
    path = Path(guest_dir)
    if not path.is_dir():
        click.echo(f"error: directory not found: {guest_dir}", err=True)
        raise SystemExit(1)

    artifacts_path = Path(artifacts) if artifacts else None
    diags = validate_guest(path, artifacts_dir=artifacts_path)

    errors = [d for d in diags if d.severity == Severity.ERROR]
    warnings = [d for d in diags if d.severity == Severity.WARNING]

    for d in diags:
        click.echo(str(d))

    if errors:
        click.echo(f"\n{len(errors)} error(s), {len(warnings)} warning(s)")
        raise SystemExit(1)

    if warnings:
        click.echo(f"\n{len(warnings)} warning(s), 0 errors -- passed")
    else:
        click.echo("passed: config is clean")


# ---------------------------------------------------------------------------
# build
# ---------------------------------------------------------------------------


@cli.command()
@click.argument("guest_dir", default="guest", type=click.Path(exists=False))
@click.option("--arch", default=None, help="Build for a single architecture only.")
@click.option("--dry-run", is_flag=True, help="Render Dockerfiles without building.")
@click.option("--json", "json_output", is_flag=True, help="Output build manifest as JSON (with --dry-run).")
@click.option("--template", default="rootfs", type=click.Choice(["rootfs", "kernel"]),
              help="Dockerfile template to render.")
def build(
    guest_dir: str,
    arch: str | None,
    dry_run: bool,
    json_output: bool,
    template: str,
) -> None:
    """Build guest images from config."""
    path = Path(guest_dir)
    if not path.is_dir():
        click.echo(f"error: directory not found: {guest_dir}", err=True)
        raise SystemExit(1)

    # Validate first
    diags = validate_guest(path)
    errors = [d for d in diags if d.severity == Severity.ERROR]
    if errors:
        for d in errors:
            click.echo(str(d), err=True)
        click.echo(f"\n{len(errors)} validation error(s) -- fix before building", err=True)
        raise SystemExit(1)

    config = load_guest_config(path)
    template_name = f"Dockerfile.{template}.j2"

    # Determine architectures
    arches = list(config.build.architectures.keys())
    if arch:
        if arch not in config.build.architectures:
            click.echo(
                f"error: architecture '{arch}' not in config "
                f"(available: {', '.join(arches)})",
                err=True,
            )
            raise SystemExit(1)
        arches = [arch]

    if dry_run:
        if json_output:
            manifest = {
                "architectures": {},
                "template": template,
                "compression": config.build.compression.value,
                "compression_level": config.build.compression_level,
            }
            for arch_name in arches:
                rendered = render_dockerfile(template_name, config, arch_name)
                manifest["architectures"][arch_name] = {
                    "dockerfile": rendered,
                    "platform": config.build.architectures[arch_name].docker_platform,
                    "rust_target": config.build.architectures[arch_name].rust_target,
                }
            click.echo(json.dumps(manifest, indent=2))
        else:
            for arch_name in arches:
                if len(arches) > 1:
                    click.echo(f"# --- {arch_name} ({template}) ---")
                rendered = render_dockerfile(template_name, config, arch_name)
                click.echo(rendered)
    else:
        click.echo("error: docker build not yet implemented (use --dry-run to preview Dockerfiles)", err=True)
        raise SystemExit(1)


# ---------------------------------------------------------------------------
# inspect
# ---------------------------------------------------------------------------


@cli.command()
@click.argument("guest_dir", default="guest", type=click.Path(exists=False))
@click.option("--json", "json_output", is_flag=True, help="Output as JSON.")
def inspect(guest_dir: str, json_output: bool) -> None:
    """Show guest config summary."""
    path = Path(guest_dir)
    if not path.is_dir():
        click.echo(f"error: directory not found: {guest_dir}", err=True)
        raise SystemExit(1)

    try:
        config = load_guest_config(path)
    except Exception as e:
        click.echo(f"error: failed to load config: {e}", err=True)
        raise SystemExit(1)

    if json_output:
        data = config.model_dump(mode="json")
        click.echo(json.dumps(data, indent=2))
        return

    # Human-readable summary
    click.echo("Build")
    click.echo(f"  compression: {config.build.compression.value} (level {config.build.compression_level})")
    click.echo(f"  architectures:")
    for name, arch in config.build.architectures.items():
        click.echo(f"    {name}: {arch.docker_platform} ({arch.rust_target})")

    if config.ai_providers:
        click.echo("\nAI Providers")
        for key, prov in config.ai_providers.items():
            status = "enabled" if prov.enabled else "disabled"
            click.echo(f"  {key}: {prov.name} [{status}]")
            click.echo(f"    domains: {', '.join(prov.network.domains)}")

    if config.package_sets:
        click.echo("\nPackage Sets")
        for key, ps in config.package_sets.items():
            click.echo(f"  {key}: {ps.manager.value} ({len(ps.packages)} packages)")

    if config.mcp_servers:
        click.echo("\nMCP Servers")
        for key, server in config.mcp_servers.items():
            click.echo(f"  {key}: {server.name} ({server.transport.value})")

    res = config.vm_resources
    click.echo(f"\nVM Resources")
    click.echo(f"  cpu: {res.cpu_count} cores, ram: {res.ram_gb} GB, disk: {res.scratch_disk_size_gb} GB")


# ---------------------------------------------------------------------------
# audit
# ---------------------------------------------------------------------------


@cli.command()
@click.option("--scanner", default="trivy", type=click.Choice(["trivy", "grype"]),
              help="Vulnerability scanner format.")
@click.option("--input", "input_file", type=click.Path(exists=True), default=None,
              help="Read scanner JSON from file (default: stdin).")
@click.option("--json", "json_output", is_flag=True, help="Output as JSON.")
def audit(scanner: str, input_file: str | None, json_output: bool) -> None:
    """Parse vulnerability scan results."""
    from capsem.builder.audit import parse_audit_output, summarize_vulns

    if input_file:
        text = Path(input_file).read_text()
    else:
        text = click.get_text_stream("stdin").read()

    if not text.strip():
        click.echo("error: no input (provide --input or pipe via stdin)", err=True)
        raise SystemExit(1)

    try:
        vulns = parse_audit_output(text, scanner)
    except ValueError as e:
        click.echo(f"error: {e}", err=True)
        raise SystemExit(1)

    if json_output:
        click.echo(json.dumps([v.model_dump() for v in vulns], indent=2))
    else:
        summary = summarize_vulns(vulns)
        click.echo(f"Scanner: {scanner}")
        click.echo(f"Total: {len(vulns)} vulnerabilities")
        for sev in ("CRITICAL", "HIGH", "MEDIUM", "LOW", "UNKNOWN"):
            if summary[sev]:
                click.echo(f"  {sev}: {summary[sev]}")
        if vulns:
            click.echo("")
            for v in vulns:
                fixed = f" (fix: {v.fixed_version})" if v.fixed_version else ""
                click.echo(f"  {v.severity:8s}  {v.id:20s}  {v.package} {v.installed_version}{fixed}")

    summary = summarize_vulns(vulns)
    if summary["CRITICAL"] or summary["HIGH"]:
        raise SystemExit(1)


# ---------------------------------------------------------------------------
# mcp
# ---------------------------------------------------------------------------


@cli.command("mcp")
def mcp_cmd() -> None:
    """Start MCP stdio server for builder tools."""
    from capsem.builder.mcp_server import run_mcp_server
    run_mcp_server()


# ---------------------------------------------------------------------------
# init
# ---------------------------------------------------------------------------


@cli.command()
@click.argument("target", default="guest", type=click.Path())
@click.option("--force", is_flag=True, help="Overwrite existing config directory.")
def init(target: str, force: bool) -> None:
    """Scaffold a new guest config directory."""
    try:
        init_guest_dir(Path(target), force=force)
    except FileExistsError as e:
        click.echo(f"error: {e}", err=True)
        raise SystemExit(1)
    click.echo(f"created {target}/config/")


# ---------------------------------------------------------------------------
# add (sub-group)
# ---------------------------------------------------------------------------


@cli.group()
def add() -> None:
    """Add config templates (AI provider, packages, MCP server)."""


@add.command("ai-provider")
@click.argument("name")
@click.option("--dir", "guest_dir", default="guest", type=click.Path(),
              help="Guest directory.")
@click.option("--force", is_flag=True, help="Overwrite existing file.")
def add_ai(name: str, guest_dir: str, force: bool) -> None:
    """Add an AI provider template."""
    try:
        path = add_ai_provider(Path(guest_dir), name, force=force)
    except (FileExistsError, FileNotFoundError) as e:
        click.echo(f"error: {e}", err=True)
        raise SystemExit(1)
    click.echo(f"created {path}")


@add.command("packages")
@click.argument("name")
@click.option("--dir", "guest_dir", default="guest", type=click.Path(),
              help="Guest directory.")
@click.option("--manager", default="apt",
              type=click.Choice(["apt", "uv", "pip", "npm"]),
              help="Package manager.")
@click.option("--force", is_flag=True, help="Overwrite existing file.")
def add_pkg(name: str, guest_dir: str, manager: str, force: bool) -> None:
    """Add a package set template."""
    try:
        path = add_package_set(Path(guest_dir), name, manager=manager, force=force)
    except (FileExistsError, FileNotFoundError) as e:
        click.echo(f"error: {e}", err=True)
        raise SystemExit(1)
    click.echo(f"created {path}")


@add.command("mcp")
@click.argument("name")
@click.option("--dir", "guest_dir", default="guest", type=click.Path(),
              help="Guest directory.")
@click.option("--transport", default="stdio",
              type=click.Choice(["stdio", "sse"]),
              help="MCP transport type.")
@click.option("--force", is_flag=True, help="Overwrite existing file.")
def add_mcp(name: str, guest_dir: str, transport: str, force: bool) -> None:
    """Add an MCP server template."""
    try:
        path = add_mcp_server(Path(guest_dir), name, transport=transport, force=force)
    except (FileExistsError, FileNotFoundError) as e:
        click.echo(f"error: {e}", err=True)
        raise SystemExit(1)
    click.echo(f"created {path}")


def main() -> None:
    """Entry point for capsem-builder."""
    cli()
