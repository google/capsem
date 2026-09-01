"""Native-runtime commands registered on the public cache CLI."""

from __future__ import annotations

import click

from .capacity import ensure_capacity
from .config import load_policy
from .failureartifacts import capture_failure as capture_failure_bundle
from .paths import CachePaths
from .runtimeinventory import scan_runtimes
from .runtimeoperations import apply_runtime_prune
from .runtimeplanner import plan_release, plan_repository_reclaim, plan_runtime_prune


def _state(
    context: click.Context,
    *,
    runtime_id: str | None = None,
    docker_control: bool = False,
):
    root = context.obj["repository"]
    policy = load_policy(root)
    paths = CachePaths(repository_root=root, policy=policy)
    if docker_control:
        if policy.control is None:
            raise click.ClickException("cache policy has no native control configuration")
        runtime_id = policy.control.docker.runtime_id
    selected = None if runtime_id is None else frozenset({runtime_id})
    snapshot = scan_runtimes(policy, runtime_ids=selected)
    return policy, paths, snapshot


def _apply(paths, policy, plan, *, apply: bool, reason: str) -> None:
    if plan.violations:
        raise click.ClickException("; ".join(plan.violations))
    if not apply:
        click.echo(plan.model_dump_json(indent=2))
        return
    result = apply_runtime_prune(paths, policy, plan, reason=reason)
    failures = [item.output for item in result.results if item.returncode != 0]
    if failures:
        raise click.ClickException("native cache mutation failed: " + "; ".join(failures))
    click.echo(f"APPLIED native: {len(result.results)} actions")


@click.command("reclaim-image")
@click.argument("resource_id")
@click.option("--keep", required=True)
@click.option("--protect", multiple=True)
@click.option("--apply", is_flag=True)
@click.option("--reason", default="superseded image generation")
@click.pass_context
def reclaim_image(context, resource_id, keep, protect, apply, reason) -> None:
    """Retire superseded tags only around a present exact anchor."""
    policy, paths, snapshot = _state(context, docker_control=True)
    try:
        plan = plan_repository_reclaim(snapshot, policy, resource_id, keep=keep, protect=protect)
    except ValueError as error:
        raise click.ClickException(str(error)) from error
    _apply(paths, policy, plan, apply=apply, reason=reason)


@click.command("release")
@click.argument("boundary")
@click.option("--apply", is_flag=True)
@click.option("--reason", default="final cache consumer completed")
@click.pass_context
def release_boundary(context, boundary, apply, reason) -> None:
    """Release exact working images at one lifetime boundary."""
    policy, paths, snapshot = _state(context, docker_control=True)
    try:
        plan = plan_release(snapshot, policy, boundary)
    except ValueError as error:
        raise click.ClickException(str(error)) from error
    _apply(paths, policy, plan, apply=apply, reason=reason)


@click.command("ensure-space")
@click.argument("rail")
@click.option("--reason", default="build rail preflight")
@click.pass_context
def ensure_space(context, rail, reason) -> None:
    """Prove Docker daemon headroom, retaining hot BuildKit layers."""
    root = context.obj["repository"]
    policy = load_policy(root)
    paths = CachePaths(repository_root=root, policy=policy)
    try:
        decision = ensure_capacity(paths, policy, rail, reason=reason)
    except ValueError as error:
        raise click.ClickException(str(error)) from error
    click.echo(decision.model_dump_json(indent=2))
    if decision.violations:
        raise click.ClickException("; ".join(decision.violations))


@click.command("capture-failure")
@click.option("--label", required=True)
@click.option("--run-id")
@click.option("--source-commit")
@click.option("--offline", is_flag=True)
@click.pass_context
def capture_failure(context, label, run_id, source_commit, offline) -> None:
    """Preserve bounded typed evidence after an expensive failure."""
    root = context.obj["repository"]
    policy = load_policy(root)
    paths = CachePaths(repository_root=root, policy=policy)
    try:
        destination = capture_failure_bundle(
            paths,
            policy,
            label=label,
            run_id=run_id,
            source_commit=source_commit,
            offline=offline,
        )
    except ValueError as error:
        raise click.ClickException(str(error)) from error
    click.echo(destination)


@click.command("runtime-status")
@click.argument("runtime_id")
@click.pass_context
def runtime_status(context, runtime_id) -> None:
    """Emit one strict native runtime inventory."""
    _, _, snapshot = _state(context, runtime_id=runtime_id)
    click.echo(snapshot.model_dump_json(indent=2))


@click.command("runtime-prune")
@click.argument("runtime_id")
@click.option("--apply", is_flag=True)
@click.option("--reason", default="native runtime retention")
@click.pass_context
def runtime_prune(context, runtime_id, apply, reason) -> None:
    """Apply routine retention to one explicitly selected runtime."""
    policy, paths, snapshot = _state(context, runtime_id=runtime_id)
    plan = plan_runtime_prune(snapshot, policy)
    _apply(paths, policy, plan, apply=apply, reason=reason)


@click.command("policy")
@click.pass_context
def show_policy(context) -> None:
    """Emit the validated policy used by every cache producer."""
    click.echo(load_policy(context.obj["repository"]).model_dump_json(indent=2))


def register(group: click.Group) -> None:
    """Register native commands without growing the generic CLI module."""
    for command in (
        reclaim_image,
        release_boundary,
        ensure_space,
        capture_failure,
        runtime_status,
        runtime_prune,
        show_policy,
    ):
        group.add_command(command)
