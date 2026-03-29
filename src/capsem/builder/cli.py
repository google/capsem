"""Capsem builder CLI -- config-driven guest image tooling.

Commands:
  doctor    Check build prerequisites
  validate  Lint and validate guest config
  build     Render Dockerfiles (--dry-run) or build images
  inspect   Show config summary
  init      Scaffold a new guest config directory
  add       Add AI provider, package set, or MCP server templates
"""

from __future__ import annotations

import json
from pathlib import Path

import click

from capsem.builder.config import load_guest_config
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
# doctor
# ---------------------------------------------------------------------------


@cli.command()
@click.argument("guest_dir", default="guest", type=click.Path(exists=False))
def doctor(guest_dir: str) -> None:
    """Check build prerequisites (container runtime, Rust, tools)."""
    from capsem.builder.doctor import format_results, run_all_checks

    guest_path = Path(guest_dir)
    repo_root = Path.cwd()
    results = run_all_checks(guest_path, repo_root)
    click.echo(format_results(results))
    failures = [r for r in results if not r.passed]
    if failures:
        raise SystemExit(1)


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
@click.option("--output", "output_dir", default="assets", type=click.Path(),
              help="Output directory for built assets (default: assets/).")
@click.option("--kernel-version", default=None,
              help="Explicit kernel version (skips auto-detection from kernel.org).")
def build(
    guest_dir: str,
    arch: str | None,
    dry_run: bool,
    json_output: bool,
    template: str,
    output_dir: str,
    kernel_version: str | None,
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
        import subprocess

        from capsem.builder.docker import (
            build_all_architectures,
            build_image,
            detect_runtime,
        )

        try:
            runtime = detect_runtime()
        except RuntimeError as e:
            click.echo(f"error: {e}", err=True)
            raise SystemExit(1)

        click.echo(f"Using container runtime: {runtime}")
        out = Path(output_dir)

        try:
            if arch:
                build_image(
                    config, arch,
                    template=template,
                    output_dir=out,
                    kernel_version=kernel_version,
                )
            else:
                build_all_architectures(
                    config,
                    template=template,
                    output_dir=out,
                    kernel_version=kernel_version,
                )
        except subprocess.CalledProcessError as e:
            click.echo(f"error: build command failed: {e.cmd}", err=True)
            raise SystemExit(1)
        except RuntimeError as e:
            click.echo(f"error: {e}", err=True)
            raise SystemExit(1)
        finally:
            # Prune dangling images from multi-stage builds
            from capsem.builder.docker import run_cmd
            try:
                run_cmd([runtime, "image", "prune", "-f"], capture=True)
            except RuntimeError:
                pass

        click.echo(f"\nDone! Assets are in {out}/")


# ---------------------------------------------------------------------------
# agent
# ---------------------------------------------------------------------------


@cli.command()
@click.argument("guest_dir", default="guest", type=click.Path(exists=False))
@click.option("--arch", default=None, help="Build for a single architecture only.")
@click.option("--output", "output_dir", default="target/linux-agent", type=click.Path(),
              help="Output directory for agent binaries.")
def agent(
    guest_dir: str,
    arch: str | None,
    output_dir: str,
) -> None:
    """Compile guest agent binaries (native or container-based)."""
    path = Path(guest_dir)
    if not path.is_dir():
        click.echo(f"error: directory not found: {guest_dir}", err=True)
        raise SystemExit(1)

    config = load_guest_config(path)
    repo_root = Path.cwd()

    # Default to host architecture
    import os
    import sys

    host_arch = "arm64" if os.uname().machine in ("arm64", "aarch64") else "x86_64"
    arch_name = arch or host_arch

    if arch_name not in config.build.architectures:
        click.echo(f"error: architecture '{arch_name}' not in config", err=True)
        raise SystemExit(1)

    rust_target = config.build.architectures[arch_name].rust_target
    out = Path(output_dir) / arch_name

    from capsem.builder.docker import cross_compile_agent
    try:
        cross_compile_agent(rust_target, repo_root, out)
    except Exception as e:
        click.echo(f"error: {e}", err=True)
        raise SystemExit(1)

    click.echo(f"Done! Agent binaries for {arch_name} are in {out}/")


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
    if config.manifest:
        click.echo(f"Image: {config.manifest.name} v{config.manifest.version}")
        if config.manifest.description:
            click.echo(f"  {config.manifest.description}")
        click.echo("")

    click.echo("Build")
    click.echo(f"  compression: {config.build.compression.value} (level {config.build.compression_level})")
    click.echo("  architectures:")
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
    click.echo("\nVM Resources")
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
# new
# ---------------------------------------------------------------------------


def _select_items(
    label: str,
    items: dict[str, str],
    interactive: bool,
) -> list[str]:
    """Present a numbered list and ask user to select items."""
    if not items:
        return []
    if not interactive:
        return list(items.keys())

    keys = list(items.keys())
    click.echo(f"\n{label} -- include from base:")
    for i, (key, desc) in enumerate(items.items(), 1):
        click.echo(f"  [{i}] {key} -- {desc}")
    raw = click.prompt("Include (comma-separated, * for all)", default="*")
    if raw.strip() == "*":
        return keys
    selected = []
    for part in raw.split(","):
        part = part.strip()
        if part.isdigit():
            idx = int(part) - 1
            if 0 <= idx < len(keys):
                selected.append(keys[idx])
        elif part in keys:
            selected.append(part)
    return selected


def _prompt_add_new(
    label: str,
    guest_dir: Path,
    add_func,
    interactive: bool,
    **kwargs,
) -> None:
    """Ask user if they want to add new items from templates."""
    if not interactive:
        return
    while click.confirm(f"Add a new {label}?", default=False):
        name = click.prompt(f"  {label} name")
        try:
            path = add_func(guest_dir, name, force=True, **kwargs)
            click.echo(f"  -> Created {path.relative_to(guest_dir)} from template")
        except Exception as e:
            click.echo(f"  error: {e}", err=True)


@cli.command("new")
@click.argument("target", type=click.Path())
@click.option("--from", "base_dir", default="guest", type=click.Path(exists=True),
              help="Base config to copy from (default: guest/).")
@click.option("--non-interactive", is_flag=True, help="Copy all from base, no prompts.")
@click.option("--force", is_flag=True, help="Overwrite existing config directory.")
def new_cmd(target: str, base_dir: str, non_interactive: bool, force: bool) -> None:
    """Create a new image config from a base config."""
    from capsem.builder.scaffold import (
        add_ai_provider,
        add_mcp_server,
        add_package_set,
        new_image,
        scan_base_config,
    )

    base = Path(base_dir)
    target_path = Path(target)
    interactive = not non_interactive

    # Image metadata
    if interactive:
        name = click.prompt("Image name", default=target_path.name)
        version = click.prompt("Version", default="0.1.0")
        description = click.prompt("Description", default="")
    else:
        name = target_path.name
        version = "0.1.0"
        description = ""

    # Scan base config for available components
    scan = scan_base_config(base)
    click.echo(f"\nScanning base config ({base_dir})...")

    # Select components
    providers = _select_items("AI Providers", scan["providers"], interactive)
    packages = _select_items("Package Sets", scan["packages"], interactive)
    mcp = _select_items("MCP Servers", scan["mcp"], interactive)

    if interactive and scan["has_security"]:
        include_security = click.confirm("Include security config?", default=True)
    else:
        include_security = scan["has_security"]

    if interactive and scan["has_vm"]:
        include_vm = click.confirm("Include VM resources/environment?", default=True)
    else:
        include_vm = scan["has_vm"]

    try:
        config_dir = new_image(
            target_path, base,
            name=name,
            version=version,
            description=description,
            include_providers=providers,
            include_packages=packages,
            include_mcp=mcp,
            include_security=include_security,
            include_vm=include_vm,
            force=force,
        )
    except FileExistsError as e:
        click.echo(f"error: {e}", err=True)
        raise SystemExit(1)

    # Phase 2: add new items from templates
    if interactive:
        _prompt_add_new("AI provider", target_path, add_ai_provider, interactive)
        _prompt_add_new("package set", target_path, add_package_set, interactive,
                        manager="apt")
        _prompt_add_new("MCP server", target_path, add_mcp_server, interactive)

    file_count = sum(1 for _ in config_dir.rglob("*.toml"))
    click.echo(f"\nCreated {config_dir}/ ({file_count} files)")


# ---------------------------------------------------------------------------
# init
# ---------------------------------------------------------------------------


@cli.command()
@click.argument("target", default="guest", type=click.Path())
@click.option("--force", is_flag=True, help="Overwrite existing config directory.")
def init(target: str, force: bool) -> None:
    """Scaffold a new guest config directory (minimal, from template)."""
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
