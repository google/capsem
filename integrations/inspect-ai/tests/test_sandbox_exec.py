"""`CapsemSandboxEnvironment` exec, file I/O, and self-check tests."""

from __future__ import annotations

import os
import pwd
import shlex
import subprocess
from pathlib import Path
from typing import Any, cast

import inspect_capsem._exec as exec_mod
import inspect_capsem._files as files_mod
import inspect_capsem.sandbox as sb
import pytest
from inspect_ai.util import OutputLimitExceededError
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment
from inspect_capsem._controller import CommandResult

from .helpers import (
    LocalFakeCapsemController,
    Scripted,
    _host_timeout_skips,
    _run_inspect_self_check,
    env_for,
    fail,
    ok,
)


async def test_read_file_limits_and_decoding(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    ctrl = Scripted([("SIZE:", ok("SIZE:5000\n"))])
    env = env_for(ctrl)
    monkeypatch.setattr(files_mod.SandboxEnvironmentLimits, "MAX_READ_FILE_SIZE", 100)
    with pytest.raises(OutputLimitExceededError):
        await env.read_file("big")
    ctrl.rules = [("SIZE:", ok("SIZE:\n"))]
    ctrl.files["/workspace/big"] = b"x" * 200
    with pytest.raises(OutputLimitExceededError):
        await env.read_file("big")
    assert ctrl.download_max_bytes == [100]
    ctrl.files["/workspace/bin"] = b"\xff\xfe"
    with pytest.raises(UnicodeDecodeError):
        await env.read_file("bin")
    ctrl.files["/workspace/nul"] = b"a\x00b"
    with pytest.raises(UnicodeDecodeError):
        await env.read_file("nul")
    ctrl.files["/workspace/ok"] = b"fine"
    assert await env.read_file("ok") == "fine"

    local_ctrl = LocalFakeCapsemController(tmp_path)
    local_env = env_for(local_ctrl, working_dir=str(tmp_path))
    fifo_path = tmp_path / "special.fifo"
    os.mkfifo(fifo_path)
    with pytest.raises(PermissionError, match="Not a regular file"):
        await local_env.read_file("/dev/zero")
    with pytest.raises(PermissionError, match="Not a regular file"):
        await local_env.read_file(str(fifo_path))


async def test_exec_edge_cases() -> None:
    ctrl = Scripted()
    env = env_for(ctrl)
    assert (await env.exec([])).success
    ctrl.rules = [("./tool", fail(126))]
    assert not (await env.exec(["./tool"])).success
    ctrl.rules = [("my_script", fail(126, stderr="custom failure"))]
    assert (await env.exec(["bash", "-c", "my_script"])).returncode == 126
    ctrl.rules = [("timeout -k", fail(126, stderr="bash: Permission denied"))]
    with pytest.raises(PermissionError, match="Permission denied"):
        await env.exec(["tool"], timeout=5)
    ctrl.rules = [("timeout -k", CommandResult(0, "x", "", truncated=True))]
    with pytest.raises(OutputLimitExceededError):
        await env.exec(["tool"], timeout=5)

    ctrl.rules = []
    ctrl.commands.clear()
    result = await env.exec(["cat"], input="x" * 20000, env={"A": "1"}, user="bob")
    assert result.success
    assert any(".capsem_stdin_" in p for p in ctrl.uploads)
    assert len(ctrl.commands) == 1
    script = ctrl.commands[0]
    assert "su -m bob" in script and "export A=1" in script and "timeout -k" not in script
    assert "rm -f /tmp/.capsem_stdin_" in script
    await env.exec(["echo", "y" * 70000])
    assert any(".capsem_cmd_" in p for p in ctrl.uploads)
    assert len(ctrl.commands) == 2 and "rm -f /tmp/.capsem_cmd_" in ctrl.commands[1]

    ctrl.commands.clear()
    env_nonroot = env_for(ctrl, user="developer")
    assert (await env_nonroot.exec(["id", "-un"])).success
    assert any(
        "su -m developer" in c
        and 'export USER="$__u" LOGNAME="$__u" HOME="${__h:-/home/$__u}"' in c
        for c in ctrl.commands
    )
    nonroot_script, _ = exec_mod._format_exec_command(
        'printf "%s:%s:%s" "$USER" "$LOGNAME" "$HOME"',
        effective_cwd="/",
        env=None,
        user=str(os.getuid()),
        timeout=None,
    )
    su_idx = nonroot_script.index("-s /bin/bash -c ") + len("-s /bin/bash -c ")
    su_body = shlex.split(nonroot_script[su_idx:])[0]
    sub_env = {**os.environ, "HOME": "/root", "USER": "root", "LOGNAME": "root"}
    sub_out = subprocess.run(
        ["bash", "-c", su_body], env=sub_env, capture_output=True, text=True, check=True
    ).stdout
    uid = os.getuid()
    expected = {f"{uid}:{uid}:/home/{uid}"}
    try:
        pw = pwd.getpwuid(uid)
        expected.add(f"{pw.pw_name}:{pw.pw_name}:{pw.pw_dir}")
        expected.add(f"{pw.pw_name}:{pw.pw_name}:/home/{pw.pw_name}")
    except KeyError:
        pass
    assert sub_out in expected

    override_script, _ = exec_mod._format_exec_command(
        'printf "%s" "$HOME"',
        effective_cwd="/",
        env={"HOME": "/custom/override"},
        user=str(os.getuid()),
        timeout=None,
    )
    su_override_idx = override_script.index("-s /bin/bash -c ") + len("-s /bin/bash -c ")
    su_override_body = shlex.split(override_script[su_override_idx:])[0]
    override_out = subprocess.run(
        ["bash", "-c", su_override_body],
        env=sub_env,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    assert override_out == "/custom/override"

    with pytest.raises(ValueError, match="Invalid environment variable name"):
        exec_mod._format_exec_command(
            "echo", effective_cwd="/", env={"BAD-VAR": "1"}, user=None, timeout=None
        )
    with pytest.raises(ValueError, match="Invalid environment variable name"):
        exec_mod._format_exec_command(
            "echo", effective_cwd="/", env={"123": "1"}, user=None, timeout=None
        )

    ctrl.commands.clear()
    env_uid = env_for(ctrl, user="1000:1000")
    assert (await env_uid.exec(["id", "-u"])).success
    assert any(
        "setpriv --reuid=1000 --regid=1000 --clear-groups /bin/bash -c" in c for c in ctrl.commands
    )
    ctrl.commands.clear()
    env_bare_uid = env_for(ctrl, user="1000")
    assert (await env_bare_uid.exec(["id", "-u"])).success
    assert any(
        "id -un 1000" in c and "setpriv --reuid=1000 --regid=0 --clear-groups /bin/bash -c" in c
        for c in ctrl.commands
    )

    class Slow(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            if "timeout -k" in command:
                raise TimeoutError(f"timed out after {timeout}s")
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    slow_ctrl = Slow()
    with pytest.raises(TimeoutError):
        await env_for(slow_ctrl).exec(["sleep", "9"], input="x" * 20000, timeout=1)
    assert any(c.startswith("rm -f /tmp/.capsem_stdin_") for c in slow_ctrl.commands)


async def test_write_file_refusals() -> None:
    ctrl = Scripted([("IS_DIR", ok("IS_DIR"))])
    with pytest.raises(IsADirectoryError):
        await env_for(ctrl).write_file("d", "x")
    ctrl = Scripted([("NO_WRITE", CommandResult(13, "", ""))])
    with pytest.raises(PermissionError):
        await env_for(ctrl).write_file("f", b"x")


async def test_exit_137_sigkill_vs_actual_timeout(tmp_path: Path) -> None:
    """Exit 137 from SIGKILL is not misreported as TimeoutError."""
    controller = LocalFakeCapsemController(tmp_path)
    work_dir = tmp_path / "work"
    work_dir.mkdir()
    env = CapsemSandboxEnvironment(
        vm_id="vm-sigkill",
        controller=cast(Any, controller),
        working_dir=str(work_dir),
    )
    try:
        res = await env.exec(["bash", "-c", "kill -KILL $$"], timeout=30)
        assert res.success is False
        assert res.returncode == 137

        with pytest.raises(TimeoutError):
            await env.exec(["bash", "-c", "trap '' TERM; sleep 5"], timeout=1)
    finally:
        await env.cleanup()


async def test_exec_timeout_none_uses_service_ceiling_and_clean_error_message() -> None:
    """exec(timeout=None) uses EXEC_TIMEOUT_CEILING_SECS (3600s) and never formats 'after Nones'."""
    recorded: list[tuple[str, int]] = []
    raise_mode: str | None = None

    class TimeoutRecordingController:
        async def stop_vm(self, vm_id: str) -> None:
            del vm_id

        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            del vm_id
            recorded.append((command, timeout))
            if raise_mode == "timeout_error":
                raise TimeoutError("capsem gateway timeout")
            return CommandResult(exit_code=0, stdout="hi\n", stderr="")

    ctrl = TimeoutRecordingController()
    env = CapsemSandboxEnvironment(
        vm_id="vm-timeout-none",
        controller=cast(Any, ctrl),
        working_dir="/workspace",
    )
    try:
        res = await env.exec(["echo", "hi"], timeout=None)
        assert res.success is True
        assert res.stdout == "hi\n"
        assert len(recorded) == 1
        cmd_sent, timeout_sent = recorded[0]
        assert timeout_sent == 3600
        assert "timeout -k 1s" not in cmd_sent

        raise_mode = "timeout_error"
        with pytest.raises(TimeoutError, match=r"^Command timed out after 3600s$") as exc_info:
            await env.exec(["sleep", "9999"], timeout=None)
        assert "Nones" not in str(exc_info.value)
    finally:
        await env.cleanup()


async def test_self_check_with_local_fake_controller(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """sample_init passes self_check with LocalFakeCapsemController."""
    controller = LocalFakeCapsemController(tmp_path, skip_bake=False)
    monkeypatch.setattr(sb, "SdkCapsemController", lambda: controller)

    guest_work = tmp_path / "guest_work"
    guest_work.mkdir()
    envs = await CapsemSandboxEnvironment.sample_init(
        task_name="self_check_no_host_ws",
        config=CapsemSandboxConfig(working_dir=str(guest_work)),
        metadata={},
    )
    env = envs["default"]
    assert isinstance(env, CapsemSandboxEnvironment)
    try:
        skip = {
            "test_read_and_write_large_file_binary",
            "test_exec_input_large",
            "test_read_file_limit",
            "test_exec_as_user",
        } | _host_timeout_skips()
        results = await _run_inspect_self_check(env, skip=skip)
        failures = {k: v for k, v in results.items() if v is not True}
        assert failures == {}, f"Inspect self_check failures: {failures}"
    finally:
        await CapsemSandboxEnvironment.sample_cleanup(
            task_name="self_check_no_host_ws",
            config=None,
            environments=envs,
            interrupted=False,
        )
