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
from .runtimeinventory import scan_runtimes, write_receipts
from .runtimeoperations import apply_runtime_prune
from .runtimeplanner import plan_runtime_prune


def _state(repository: Path, *, native: bool = False):
    root = repository.resolve()
    policy = load_policy(root)
    paths = CachePaths(repository_root=root, policy=policy)
    report = scan_inventory(paths, policy)
    snapshot = scan_runtimes(policy, offline=not native)
    report = report.model_copy(update={"runtimes": snapshot.runtimes})
    return policy, paths, report, snapshot


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
@click.option("--offline", is_flag=True, help="Do not query native runtimes.")
@click.pass_context
def status(context: click.Context, as_json: bool, offline: bool) -> None:
    """Show total and per-stage cache inventory."""
    _, _, report, _ = _state(context.obj["repository"], native=not offline)
    click.echo(report.model_dump_json(indent=2) if as_json else inventory_text(report))


@main.command()
@click.option("--json", "as_json", is_flag=True, help="Emit the typed JSON result.")
@click.pass_context
def verify(context: click.Context, as_json: bool) -> None:
    """Validate policy, path containment, and inventory accounting."""
    policy, paths, report, _ = _state(context.obj["repository"])
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
    policy, paths, report, snapshot = _state(context.obj["repository"], native=True)
    plan = plan_prune(report, policy)
    runtime_plan = plan_runtime_prune(snapshot, policy)
    if apply:
        apply_prune(paths.root, plan, reason=reason)
        result = apply_runtime_prune(paths, policy, runtime_plan, reason=reason)
        if any(item.returncode != 0 for item in result.results):
            raise click.ClickException("one or more native cache mutations failed; inspect the log")
    if as_json:
        click.echo(
            json.dumps(
                {
                    "filesystem": plan.model_dump(mode="json"),
                    "runtimes": runtime_plan.model_dump(mode="json"),
                },
                indent=2,
            )
        )
    else:
        click.echo(plan_text(plan, preview=not apply))
        click.echo(
            f"{'PREVIEW' if not apply else 'APPLIED'} native: reclaim "
            f"{runtime_plan.reclaim_bytes} bytes in {len(runtime_plan.actions)} actions"
        )
        for violation in runtime_plan.violations:
            click.echo(f"  VIOLATION: {violation}")


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
    _, paths, report, _ = _state(context.obj["repository"])
    try:
        plan = plan_clean(report, stage_id)
    except KeyError as error:
        raise click.UsageError(str(error)) from error
    if apply:
        apply_prune(paths.root, plan, reason=reason or f"clean {stage_id}")
    click.echo(plan.model_dump_json(indent=2) if as_json else plan_text(plan, preview=not apply))


@main.command()
@click.option("--json", "as_json", is_flag=True, help="Emit the typed runtime snapshot.")
@click.pass_context
def snapshot(context: click.Context, as_json: bool) -> None:
    """Persist exact Docker/BuildKit/Tart inventory receipts."""
    policy, paths, _, report = _state(context.obj["repository"], native=True)
    receipts = write_receipts(paths, policy, report)
    if as_json:
        click.echo(report.model_dump_json(indent=2))
    else:
        click.echo(
            f"Recorded {len(receipts)} runtime receipts: native={report.native_bytes} "
            f"owned={report.owned_bytes}"
        )


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
