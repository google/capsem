"""Command formatting, script preparation, and execution helpers for Capsem sandboxes."""

from __future__ import annotations

import base64
import contextlib
import logging
import re
import shlex
import uuid
from collections.abc import Awaitable, Callable
from typing import TYPE_CHECKING

from capsem.execution import EXEC_TIMEOUT_CEILING_SECS
from inspect_ai.util import ExecResult, OutputLimitExceededError, SandboxEnvironmentLimits

from inspect_capsem._controller import is_root_user_spec
from inspect_capsem._files import _resolve_guest_path

if TYPE_CHECKING:
    from inspect_capsem._controller import CapsemController

logger = logging.getLogger(__name__)

_TIMEOUT_SENTINEL = "__CAPSEM_INSPECT_EXEC_TIMED_OUT__"
_EXEC_TIMEOUT_MARGIN_SECS = 10
_ENV_KEY_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


def _truncate_utf8(text: str, max_bytes: int) -> tuple[str, bool]:
    encoded = text.encode("utf-8", errors="replace")
    if len(encoded) <= max_bytes:
        return text, False
    return encoded[-max_bytes:].decode("utf-8", errors="ignore"), True


def _format_exec_command(
    exec_expr: str,
    *,
    effective_cwd: str,
    env: dict[str, str] | None,
    user: str | None,
    timeout: int | None,
) -> tuple[str, int]:
    env_prefix = ""
    if env:
        for k in env:
            if not _ENV_KEY_RE.match(k):
                raise ValueError(f"Invalid environment variable name: {k!r}")
        assignments = " ".join(f"{k}={shlex.quote(str(v))}" for k, v in env.items())
        env_prefix = f"export {assignments}; "
    inner_body = f"cd {shlex.quote(effective_cwd)} && {env_prefix}{exec_expr}"
    if not is_root_user_spec(user):
        raw_user = (user or "").strip()
        target_user, _, target_group = raw_user.partition(":")
        target_user = target_user.strip() or "0"
        target_group = target_group.strip()
        user_env_reset = (
            '__u="$(id -un 2>/dev/null || true)"; __u="${__u:-$(id -u)}"; '
            '__h="$(getent passwd "$(id -u)" 2>/dev/null | cut -d: -f6)"; '
            'export USER="$__u" LOGNAME="$__u" HOME="${__h:-/home/$__u}"; '
        )
        user_body_q = shlex.quote(f"{user_env_reset}{inner_body}")
        err_user_q = shlex.quote(f"capsem: unknown user {user}")
        if target_user.isdigit() and target_group.isdigit():
            inner_body = (
                f"setpriv --reuid={target_user} --regid={target_group} "
                f"--clear-groups /bin/bash -c {user_body_q}"
            )
        elif target_user.isdigit() and not target_group:
            u_q = shlex.quote(target_user)
            inner_body = (
                f"if id -u {u_q} >/dev/null 2>&1; then "
                f'su -m "$(id -un {u_q})" -s /bin/bash -c {user_body_q}; '
                f"else setpriv --reuid={target_user} --regid=0 "
                f"--clear-groups /bin/bash -c {user_body_q}; fi"
            )
        elif not target_group:
            su_user = shlex.quote(target_user)
            inner_body = (
                f"if id -u {su_user} >/dev/null 2>&1; then "
                f"su -m {su_user} -s /bin/bash -c {user_body_q}; "
                f"else echo {err_user_q} >&2; exit 1; fi"
            )
        else:
            u_q, g_q = shlex.quote(target_user), shlex.quote(target_group)
            uid_step = (
                f"_uid={target_user}; "
                if target_user.isdigit()
                else f"_uid=$(id -u {u_q} 2>/dev/null) || {{ echo {err_user_q} >&2; exit 1; }}; "
            )
            err_grp_q = shlex.quote(f"capsem: unknown group {target_group}")
            gid_step = (
                f"_gid={target_group}; "
                if target_group.isdigit()
                else (
                    f"_gid=$(getent group {g_q} 2>/dev/null | cut -d: -f3); "
                    f'[ -n "$_gid" ] || {{ echo {err_grp_q} >&2; exit 1; }}; '
                )
            )
            inner_body = (
                f'{uid_step}{gid_step}setpriv --reuid="$_uid" --regid="$_gid" '
                f"--clear-groups /bin/bash -c {user_body_q}"
            )
    if timeout is None:
        return inner_body, EXEC_TIMEOUT_CEILING_SECS
    timeout_secs = min(int(timeout), EXEC_TIMEOUT_CEILING_SECS - _EXEC_TIMEOUT_MARGIN_SECS)
    if timeout_secs < timeout:
        logger.warning("Exec timeout %ss exceeds Capsem limit; using %ss", timeout, timeout_secs)
    timed_script = (
        f"SECONDS=0; timeout -k 1s {timeout_secs}s bash -c {shlex.quote(inner_body)}; "
        f"rc=$?; elapsed=$SECONDS; "
        f"if [ $rc -eq 124 ] || {{ [ $rc -eq 137 ] && [ $elapsed -ge {timeout_secs} ]; }}; then "
        f"echo '{_TIMEOUT_SENTINEL}' >&2; fi; exit $rc"
    )
    return timed_script, max(timeout_secs + _EXEC_TIMEOUT_MARGIN_SECS, 30)


async def _prepare_exec_expr(
    stage_bytes: Callable[[bytes, str], Awaitable[None]],
    quoted_cmd: str,
    input_data: str | bytes | None,
    temp_files: list[str],
) -> str:
    if len(quoted_cmd) > 65536:
        script_guest = f"/tmp/.capsem_cmd_{uuid.uuid4().hex[:10]}.sh"
        temp_files.append(script_guest)
        await stage_bytes(quoted_cmd.encode("utf-8"), script_guest)
        exec_expr = f"bash {shlex.quote(script_guest)}"
    else:
        exec_expr = quoted_cmd

    if input_data is not None:
        input_bytes = input_data.encode("utf-8") if isinstance(input_data, str) else input_data
        if len(input_bytes) <= 16384:
            b64_in = base64.b64encode(input_bytes).decode("ascii")
            exec_expr = f"printf '%s' {shlex.quote(b64_in)} | base64 -d | {exec_expr}"
        else:
            stdin_guest = f"/tmp/.capsem_stdin_{uuid.uuid4().hex[:10]}.dat"
            temp_files.append(stdin_guest)
            await stage_bytes(input_bytes, stdin_guest)
            exec_expr = f"{exec_expr} < {shlex.quote(stdin_guest)}"
    return exec_expr


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
) -> ExecResult[str]:
    """Execute command in Capsem sandbox VM, enforcing timeouts and limits."""
    if not cmd:
        return ExecResult(success=True, returncode=0, stdout="", stderr="")

    effective_cwd = _resolve_guest_path(cwd, working_dir) if cwd else working_dir
    quoted_cmd = " ".join(shlex.quote(arg) for arg in cmd)

    temp_files_to_clean: list[str] = []
    cleaned_inline = False
    controller_timeout: int = EXEC_TIMEOUT_CEILING_SECS
    try:

        async def stage_bytes(data: bytes, path: str) -> None:
            await controller.upload_to_vm(vm_id, path, data)

        exec_expr = await _prepare_exec_expr(
            stage_bytes, quoted_cmd, input_data, temp_files_to_clean
        )
        effective_user = user or default_user
        timed_script, controller_timeout = _format_exec_command(
            exec_expr,
            effective_cwd=effective_cwd,
            env=env_vars,
            user=effective_user,
            timeout=timeout,
        )
        if temp_files_to_clean:
            rm_targets = " ".join(shlex.quote(p) for p in temp_files_to_clean)
            timed_script = f"( {timed_script} ); __ec=$?; rm -f {rm_targets}; exit $__ec"
        res = await controller.exec_in_vm(vm_id, timed_script, timeout=controller_timeout)
        cleaned_inline = True
    except TimeoutError as exc:
        effective_timeout = timeout if timeout is not None else controller_timeout
        raise TimeoutError(f"Command timed out after {effective_timeout}s") from exc
    finally:
        if temp_files_to_clean and not cleaned_inline:
            rm_targets = " ".join(shlex.quote(p) for p in temp_files_to_clean)
            with contextlib.suppress(Exception):
                await controller.exec_in_vm(vm_id, f"rm -f {rm_targets}", timeout=15)

    stderr_str = res.stderr
    if timeout is not None and _TIMEOUT_SENTINEL in stderr_str:
        raise TimeoutError(f"Command timed out after {timeout}s")
    if res.exit_code == 126 and "permission denied" in stderr_str.lower():
        raise PermissionError(stderr_str.strip())

    limit_bytes = SandboxEnvironmentLimits.MAX_EXEC_OUTPUT_SIZE
    stdout_str, stdout_truncated = _truncate_utf8(res.stdout, limit_bytes)
    stderr_str, stderr_truncated = _truncate_utf8(stderr_str, limit_bytes)
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
