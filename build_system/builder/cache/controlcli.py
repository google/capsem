"""Native-runtime commands registered on the public cache CLI."""

from __future__ import annotations

import json

import click

from .api import CacheOperation, CacheRequest
from .config import load_policy
from .dockerimages import plan_repository_reclaim
from .failureartifacts import capture_failure as capture_failure_bundle
from .paths import CachePaths
from .registry import CacheRegistry
from .runtimeinventory import scan_runtimes
from .runtimeoperations import apply_runtime_prune


def _state(
    context: click.Context,
    *,
    runtime_id: str | None = None,
    docker_control: bool = False,
):
    root = context.obj["repository"]
    policy = load_policy(context.obj["policy_repository"])
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


@click.command("enforce")
@click.argument("cache_id")
@click.option("--reason", default="cache size enforcement")
@click.pass_context
def enforce(context, cache_id, reason) -> None:
    """Enforce one cache owner, or all repository owners, by owned size."""
    root = context.obj["repository"]
    policy = load_policy(context.obj["policy_repository"])
    paths = CachePaths(repository_root=root, policy=policy)
    request = CacheRequest(
        operation=CacheOperation.ENFORCE,
        cache_id=cache_id,
        apply=True,
        reason=reason,
    )
    try:
        decisions = CacheRegistry(paths, policy).mutate(request)
    except ValueError as error:
        raise click.ClickException(str(error)) from error
    click.echo(json.dumps([item.model_dump(mode="json") for item in decisions], indent=2))
    violations = tuple(error for item in decisions for error in item.violations)
    if violations:
        raise click.ClickException("; ".join(violations))


@click.command("capture-failure")
@click.option("--label", required=True)
@click.option("--run-id")
@click.option("--source-commit")
@click.option("--offline", is_flag=True)
@click.pass_context
def capture_failure(context, label, run_id, source_commit, offline) -> None:
    """Preserve bounded typed evidence after an expensive failure."""
    root = context.obj["repository"]
    policy = load_policy(context.obj["policy_repository"])
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


def register(group: click.Group) -> None:
    """Register native commands without growing the generic CLI module."""
    for command in (
        reclaim_image,
        enforce,
        capture_failure,
    ):
        group.add_command(command)
