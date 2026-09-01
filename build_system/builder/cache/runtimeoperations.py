"""Journaled native-runtime mutation boundary."""

from __future__ import annotations

import time

from .models import CachePolicy
from .paths import CachePaths
from .runtimeexec import CommandRunner, execute
from .runtimemodels import (
    DockerRuntimePolicy,
    RuntimeActionResult,
    RuntimeApplyResult,
    RuntimeMutationEvent,
    RuntimeOperation,
    RuntimePruneAction,
    RuntimePrunePlan,
    TartRuntimePolicy,
)


def _argv(
    action: RuntimePruneAction,
    runtime: DockerRuntimePolicy | TartRuntimePolicy,
) -> tuple[str, ...]:
    docker_operations = {
        RuntimeOperation.REMOVE_IMAGE,
        RuntimeOperation.REMOVE_CONTAINER,
        RuntimeOperation.PRUNE_BUILD_CACHE,
        RuntimeOperation.CLEAR_BUILD_CACHE,
    }
    if action.operation in docker_operations and not isinstance(runtime, DockerRuntimePolicy):
        raise ValueError(f"{action.operation} requires a Docker runtime")
    if action.operation is RuntimeOperation.DELETE_VM and not isinstance(
        runtime, TartRuntimePolicy
    ):
        raise ValueError("delete-vm requires a Tart runtime")
    if action.operation is RuntimeOperation.REMOVE_IMAGE:
        return (runtime.command, "image", "rm", action.target)
    if action.operation is RuntimeOperation.REMOVE_CONTAINER:
        return (runtime.command, "container", "rm", action.target)
    if action.operation is RuntimeOperation.PRUNE_BUILD_CACHE:
        if not isinstance(runtime, DockerRuntimePolicy):  # narrowed for the type checker
            raise AssertionError("validated Docker operation lost its runtime type")
        if action.keep_bytes is None:
            raise ValueError("BuildKit prune action omits its retained byte budget")
        command = [
            runtime.command,
            "builder",
            "prune",
            "--force",
        ]
        if action.all_unused:
            command.append("--all")
        if action.maximum_age_hours is not None:
            command.extend(("--filter", f"until={action.maximum_age_hours}h"))
        command.extend(("--reserved-space", f"{action.keep_bytes}B"))
        return tuple(command)
    if action.operation is RuntimeOperation.CLEAR_BUILD_CACHE:
        return (runtime.command, "builder", "prune", "--all", "--force")
    if action.operation is RuntimeOperation.DELETE_VM:
        return (runtime.command, "delete", action.target)
    raise ValueError(f"unsupported runtime operation: {action.operation}")


def apply_runtime_prune(
    paths: CachePaths,
    policy: CachePolicy,
    plan: RuntimePrunePlan,
    *,
    reason: str,
    runner: CommandRunner = execute,
) -> RuntimeApplyResult:
    if not reason.strip():
        raise ValueError("runtime cache mutation reason must be non-empty")
    results = []
    for action in plan.actions:
        runtime = policy.runtimes[action.runtime_id]
        command = runner(_argv(action, runtime), runtime.mutation_timeout_seconds)
        output = "\n".join(part for part in (command.stdout, command.stderr) if part)
        results.append(
            RuntimeActionResult(action=action, returncode=command.returncode, output=output)
        )
    values = tuple(results)
    if not values:
        return RuntimeApplyResult(results=(), journal=None)
    event = RuntimeMutationEvent(
        schema_id="capsem.runtime-cache-mutation.v1",
        timestamp_ns=time.time_ns(),
        plan_generated_ns=plan.generated_ns,
        reason=reason,
        results=values,
    )
    journals = {
        paths.stage(policy.runtimes[action.runtime_id].log_stage) / "runtime-mutations.jsonl"
        for action in plan.actions
    }
    for journal in journals:
        journal.parent.mkdir(parents=True, exist_ok=True)
        with journal.open("a", encoding="utf-8") as stream:
            stream.write(event.model_dump_json() + "\n")
    return RuntimeApplyResult(results=values, journal=sorted(journals)[0])
