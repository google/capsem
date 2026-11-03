"""Inspect AI command execution wrapper over `capsem_integrations._exec`."""

from __future__ import annotations

from typing import TYPE_CHECKING

from capsem_integrations._exec import (
    _CONTAINER_HOME_FIX,
    _CONTAINER_ROOT_CHECK,
    _TIMEOUT_SENTINEL,
    _format_exec_command,
    _truncate_utf8,
    _wrap_target_command,
    run_sandbox_command,
)
from inspect_ai.util import ExecResult, OutputLimitExceededError, SandboxEnvironmentLimits

if TYPE_CHECKING:
    from capsem_integrations._controller import CapsemController


async def exec_in_sandbox(
    controller: CapsemController,
    vm_id: str,
    cmd: list[str],
    working_dir: str = "/workspace",
    default_user: str | None = None,
    input_data: str | bytes | None = None,
    cwd: str | None = None,
    env_vars: dict[str, str] | None = None,
    user: str | None = None,
    timeout: int | None = None,
    *,
    is_container: bool = False,
) -> ExecResult[str]:
    """Execute command in Capsem sandbox VM or OCI workload container for Inspect AI."""
    if not cmd:
        return ExecResult(success=True, returncode=0, stdout="", stderr="")
    res = await run_sandbox_command(
        controller,
        vm_id,
        cmd,
        working_dir,
        default_user,
        input_data,
        cwd,
        env_vars,
        user,
        timeout,
        is_container=is_container,
    )
    limit_bytes = SandboxEnvironmentLimits.MAX_EXEC_OUTPUT_SIZE
    stdout_str, stdout_truncated = _truncate_utf8(res.stdout, limit_bytes)
    stderr_str, stderr_truncated = _truncate_utf8(res.stderr, limit_bytes)
    if stdout_truncated or stderr_truncated or res.truncated:
        raise OutputLimitExceededError(
            limit_str=SandboxEnvironmentLimits.MAX_EXEC_OUTPUT_SIZE_STR,
            truncated_output=stderr_str if stderr_truncated else stdout_str,
        )
    return ExecResult(
        success=(res.exit_code == 0),
        returncode=res.exit_code,
        stdout=stdout_str,
        stderr=stderr_str,
    )


__all__ = [
    "_CONTAINER_HOME_FIX",
    "_CONTAINER_ROOT_CHECK",
    "_TIMEOUT_SENTINEL",
    "_format_exec_command",
    "_truncate_utf8",
    "_wrap_target_command",
    "exec_in_sandbox",
]
