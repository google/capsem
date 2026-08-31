"""Public capsem-cache operator interface."""

from __future__ import annotations

import json
import shlex
from pathlib import Path

import click

from .config import load_policy
from .inventory import scan_inventory
from .operations import apply_prune
from .paths import CachePaths
from .planner import plan_clean, plan_prune
from .render import inventory_text, plan_text


def _state(repository: Path):
    root = repository.resolve()
    policy = load_policy(root)
    paths = CachePaths(root, policy)
    return policy, paths, scan_inventory(paths, policy)


@click.group()
@click.option(
    "--repository",
    type=click.Path(path_type=Path, file_okay=False),
    default=Path.cwd,
    show_default="current directory",
)
@click.pass_context
def main(context: click.Context, repository: Path) -> None:
    """Inspect and control Capsem's repository cache."""
    context.ensure_object(dict)
    context.obj["repository"] = repository.resolve()


@main.command()
@click.option("--json", "as_json", is_flag=True, help="Emit the typed JSON report.")
@click.pass_context
def status(context: click.Context, as_json: bool) -> None:
    """Show total and per-stage cache inventory."""
    _, _, report = _state(context.obj["repository"])
    click.echo(report.model_dump_json(indent=2) if as_json else inventory_text(report))


@main.command()
@click.option("--json", "as_json", is_flag=True, help="Emit the typed JSON result.")
@click.pass_context
def verify(context: click.Context, as_json: bool) -> None:
    """Validate policy, path containment, and inventory accounting."""
    policy, paths, report = _state(context.obj["repository"])
    for stage_id in policy.stages:
        paths.stage(stage_id).relative_to(paths.root)
    if as_json:
        click.echo(
            json.dumps(
                {"ok": True, "stages": len(report.stages), "logical_bytes": report.logical_bytes}
            )
        )
    else:
        click.echo(f"OK: {len(report.stages)} cache stages are contained and accounted")


@main.command()
@click.option("--apply", is_flag=True, help="Execute the displayed plan.")
@click.option("--reason", default="policy prune", show_default=True)
@click.option("--json", "as_json", is_flag=True, help="Emit the typed JSON plan.")
@click.pass_context
def prune(context: click.Context, apply: bool, reason: str, as_json: bool) -> None:
    """Plan deterministic policy pruning; preview unless --apply is present."""
    policy, paths, report = _state(context.obj["repository"])
    plan = plan_prune(report, policy)
    if apply:
        apply_prune(paths.root, plan, reason=reason)
    click.echo(plan.model_dump_json(indent=2) if as_json else plan_text(plan, preview=not apply))


@main.command()
@click.argument("stage_id")
@click.option("--apply", is_flag=True, help="Execute the displayed plan.")
@click.option("--reason", default="")
@click.option("--json", "as_json", is_flag=True, help="Emit the typed JSON plan.")
@click.pass_context
def clean(context: click.Context, stage_id: str, apply: bool, reason: str, as_json: bool) -> None:
    """Plan removal of one stage or all stages."""
    if apply and stage_id == "all" and not reason.strip():
        raise click.UsageError("clean all --apply requires --reason")
    _, paths, report = _state(context.obj["repository"])
    try:
        plan = plan_clean(report, stage_id)
    except KeyError as error:
        raise click.UsageError(str(error)) from error
    if apply:
        apply_prune(paths.root, plan, reason=reason or f"clean {stage_id}")
    click.echo(plan.model_dump_json(indent=2) if as_json else plan_text(plan, preview=not apply))


@main.command(hidden=True)
@click.argument("command", default="")
@click.pass_context
def dispatch(context: click.Context, command: str) -> None:
    """Parse the one safely quoted command string supplied by Just."""
    arguments = shlex.split(command) or ["status"]
    if arguments[0] == "dispatch":
        raise click.UsageError("nested cache dispatch is not allowed")
    main.main(
        args=["--repository", str(context.obj["repository"]), *arguments],
        prog_name="capsem-cache",
        standalone_mode=False,
    )
