"""Public capsem-cache operator interface."""

from __future__ import annotations

import json
from pathlib import Path

import click

from .api import CacheOperation, CacheRequest
from .config import load_policy
from .inventory import scan_inventory
from .paths import CachePaths
from .registry import CacheRegistry
from .stats import render as stats_text


def _policy_paths(context: click.Context):
    repository = context.obj["repository"]
    policy_repository = context.obj["policy_repository"]
    root = repository.resolve()
    policy = load_policy(policy_repository)
    paths = CachePaths(repository_root=root, policy=policy)
    return policy, paths


def _registry(context: click.Context) -> CacheRegistry:
    policy, paths = _policy_paths(context)
    return CacheRegistry(paths, policy)


def _mutation_output(results, *, as_json: bool) -> None:
    if as_json:
        click.echo(json.dumps([item.model_dump(mode="json") for item in results], indent=2))
    else:
        for item in results:
            verb = "APPLIED" if item.applied else "PREVIEW"
            click.echo(
                f"{verb} {item.operation} {item.cache_id}: {item.action_count} actions, "
                f"{item.reclaim_bytes} reclaimable bytes"
            )
            for violation in item.violations:
                click.echo(f"  VIOLATION: {violation}")
    violations = tuple(error for item in results for error in item.violations)
    if violations:
        raise click.ClickException("; ".join(violations))


@click.group()
@click.option(
    "--repository",
    type=click.Path(path_type=Path, file_okay=False),
    default=Path.cwd,
    show_default="current directory",
)
@click.option(
    "--policy-repository",
    type=click.Path(path_type=Path, file_okay=False),
    default=None,
    help="Read cache policy from this source tree while controlling --repository storage.",
)
@click.pass_context
def main(context: click.Context, repository: Path, policy_repository: Path | None) -> None:
    """Inspect and control Capsem's repository cache."""
    context.ensure_object(dict)
    context.obj["repository"] = repository.resolve()
    context.obj["policy_repository"] = (policy_repository or repository).resolve()


@main.command("stats")
@click.option("--json", "as_json", is_flag=True, help="Emit the typed JSON report.")
@click.option("--offline", is_flag=True, help="Do not query native runtimes.")
@click.pass_context
def stats(context: click.Context, as_json: bool, offline: bool) -> None:
    """Show current, warm, and maximum usage for every cache owner."""
    report = _registry(context).stats(offline=offline)
    click.echo(report.model_dump_json(indent=2) if as_json else stats_text(report))


@main.command("contract")
@click.argument("cache_id")
@click.pass_context
def contract(context: click.Context, cache_id: str) -> None:
    """Show one owner's typed, mechanism-independent cache contract."""
    try:
        value = _registry(context).contract(cache_id)
    except ValueError as error:
        raise click.UsageError(str(error)) from error
    click.echo(value.model_dump_json(indent=2))


@main.command()
@click.option("--json", "as_json", is_flag=True, help="Emit the typed JSON result.")
@click.pass_context
def verify(context: click.Context, as_json: bool) -> None:
    """Validate policy, path containment, and inventory accounting."""
    policy, paths = _policy_paths(context)
    report = scan_inventory(paths, policy)
    for stage_id in policy.stages:
        paths.stage(stage_id).relative_to(paths.root)
    if report.unclassified:
        names = ", ".join(entry.relative_path.as_posix() for entry in report.unclassified)
        raise click.ClickException(f"unclassified cache paths: {names}")
    if as_json:
        click.echo(
            json.dumps(
                {"ok": True, "stages": len(report.stages), "logical_bytes": report.logical_bytes}
            )
        )
    else:
        click.echo(f"OK: {len(report.stages)} cache stages are contained and accounted")


@main.command()
@click.argument("cache_id", required=False, default="all")
@click.option("--apply", is_flag=True, help="Execute the displayed plan.")
@click.option("--reason", default="policy prune", show_default=True)
@click.option("--json", "as_json", is_flag=True, help="Emit the typed JSON plan.")
@click.pass_context
def prune(context: click.Context, cache_id: str, apply: bool, reason: str, as_json: bool) -> None:
    """Plan deterministic pruning for one owner or all owners."""
    request = CacheRequest(
        operation=CacheOperation.PRUNE,
        cache_id=cache_id,
        apply=apply,
        reason=reason,
    )
    try:
        results = _registry(context).mutate(request)
    except ValueError as error:
        raise click.UsageError(str(error)) from error
    _mutation_output(results, as_json=as_json)


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
    request = CacheRequest(
        operation=CacheOperation.CLEAN,
        cache_id=stage_id,
        apply=apply,
        reason=reason or f"clean {stage_id}",
    )
    try:
        results = _registry(context).mutate(request)
    except ValueError as error:
        raise click.UsageError(str(error)) from error
    _mutation_output(results, as_json=as_json)


@main.command(hidden=True, context_settings={"ignore_unknown_options": True})
@click.argument("arguments", nargs=-1, type=click.UNPROCESSED)
@click.pass_context
def dispatch(context: click.Context, arguments: tuple[str, ...]) -> None:
    """Forward the exact argument vector supplied by Just."""
    selected = list(arguments) or ["stats"]
    if selected[0] == "dispatch":
        raise click.UsageError("nested cache dispatch is not allowed")
    main.main(
        args=["--repository", str(context.obj["repository"]), *selected],
        prog_name="capsem-cache",
        standalone_mode=False,
    )


from .controlcli import register as _register_control  # noqa: E402

_register_control(main)
