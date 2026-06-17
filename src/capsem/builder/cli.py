"""Capsem builder CLI -- backend-only helper tooling.

Product profile validation, materialization, and image builds are owned by
capsem-admin. This CLI intentionally exposes only backend helpers that are used
by just/CI and do not create a second product authoring rail.
"""

from __future__ import annotations

import json
from pathlib import Path

import click

from capsem.builder.config import load_guest_config


@click.group(invoke_without_command=True)
@click.version_option(package_name="capsem", prog_name="capsem-builder")
@click.pass_context
def cli(ctx: click.Context) -> None:
    """Capsem builder -- backend helper tooling."""
    if ctx.invoked_subcommand is None:
        click.echo(ctx.get_help())


# ---------------------------------------------------------------------------
# doctor
# ---------------------------------------------------------------------------


@cli.command()
@click.option("--profile", "profile_id", default="code", show_default=True,
              help="Profile id whose ledger should be checked.")
@click.option("--config-root", default="config", show_default=True,
              type=click.Path(exists=False),
              help="Config root containing profiles and rule files.")
def doctor(profile_id: str, config_root: str) -> None:
    """Check build prerequisites and the profile-derived build contract."""
    from capsem.builder.doctor import format_results, run_all_checks

    repo_root = Path.cwd()
    results = run_all_checks(
        repo_root,
        profile_id=profile_id,
        config_root=Path(config_root),
    )
    click.echo(format_results(results))
    failures = [r for r in results if not r.passed]
    if failures:
        raise SystemExit(1)


@cli.command("validate-skills")
@click.argument("skills_dir", default="skills", type=click.Path(exists=False))
@click.option("--json", "json_output", is_flag=True, help="Output validation report as JSON.")
def validate_skills(skills_dir: str, json_output: bool) -> None:
    """Validate the canonical Capsem skill library."""
    from capsem.builder.skills import validate_skill_library

    path = Path(skills_dir)
    try:
        report = validate_skill_library(path)
    except Exception as e:
        click.echo(f"error: {e}", err=True)
        raise SystemExit(1)

    if json_output:
        click.echo(report.model_dump_json(indent=2))
    else:
        click.echo(f"passed: {report.skill_count} skills validated in {report.root}")


# ---------------------------------------------------------------------------
# agent
# ---------------------------------------------------------------------------


@cli.command()
@click.argument("guest_dir", default="config/docker/image", type=click.Path(exists=False))
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


def main() -> None:
    """Entry point for capsem-builder."""
    cli()
