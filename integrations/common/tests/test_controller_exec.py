"""SDK controller execution, timeout forwarding, gateway error mapping, and HTTP loop tests."""

from __future__ import annotations

import os
import pwd
import shlex
import shutil
import subprocess
from pathlib import Path
from typing import Any, cast

import capsem
import capsem_integrations._exec as exec_mod
import pytest
from capsem import ExecTimeoutError, HttpError, models
from capsem_integrations._controller import (
    CommandResult,
    SdkCapsemController,
    _normalize_image_ref,
)
from capsem_integrations._exec import (
    _format_exec_command,
    _resolve_guest_path,
    _truncate_utf8,
    _wrap_target_command,
    run_sandbox_command,
)
from capsem_integrations._users import is_root_user_spec

from .conftest import write_portable_timeout_shim
from .helpers import (
    FakeManagedScope,
    LocalFakeCapsemController,
    Scripted,
    _host_timeout_skips,
    exec_response,
    fail,
)


async def test_exec_timeout_forwarding_and_service_ceiling(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    assert _normalize_image_ref(None) is None
    assert _normalize_image_ref("  ") is None
    assert _normalize_image_ref("python:3.12-slim") == "docker://python:3.12-slim"
    assert _normalize_image_ref("docker://alpine:3.19") == "docker://alpine:3.19"
    assert _resolve_guest_path("/abs/file", "/workspace") == "/abs/file"
    assert _resolve_guest_path("rel/file", "workspace") == "/workspace/rel/file"
    assert _truncate_utf8("hello", 10) == ("hello", False)
    assert _truncate_utf8("hello world", 5) == ("world", True)

    seen: list[tuple[int, str]] = []

    class Recorder(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            seen.append((timeout, command))
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    for requested, guest in ((3600, 3590), (7200, 3590), (900, 900)):
        seen.clear()
        await run_sandbox_command(Recorder(), "vm-1", ["make"], timeout=requested)
        timeout, command = next(s for s in seen if "timeout -k" in s[1])
        assert timeout == guest + 10 and timeout <= 3600
        assert f"timeout -k 1s {guest}s " in command

    calls: list[tuple[int | None, Any]] = []
    hv = capsem.Hypervisor("http://127.0.0.1:1", "t")

    def fake_managed(self: Any, **kwargs: Any) -> Any:
        del kwargs
        return FakeManagedScope(self.vm(id="s"))

    async def fake_exec(
        self: Any, command: str, *, timeout_secs: int | None = None, target: Any = None
    ) -> Any:
        del self, command
        calls.append((timeout_secs, target))
        return exec_response(0)

    monkeypatch.setattr(capsem.Hypervisor, "managed", fake_managed)
    monkeypatch.setattr(capsem.VM, "exec", fake_exec)
    ctrl = SdkCapsemController(hypervisor=hv)
    try:
        vid = await ctrl.start_vm(cpu_count=1, ram_gb=1)
        await ctrl.exec_in_vm(vid, "true", timeout=910)
        await ctrl.exec_in_vm(vid, "true", timeout=3610)
        vid_oci = await ctrl.start_vm(cpu_count=1, ram_gb=1, image="alpine:3.19")
        await ctrl.exec_in_vm(vid_oci, "true", timeout=60)
        assert calls == [
            (910, models.ExecTarget.VM),
            (3600, models.ExecTarget.VM),
            (60, models.ExecTarget.WORKLOAD),
        ]
    finally:
        await ctrl.close()


async def test_sdk_controller_maps_gateway_errors_and_timeouts(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    targets_seen: list[Any] = []

    def fake_managed(self: Any, **kwargs: Any) -> Any:
        del kwargs
        return FakeManagedScope(self.vm(id="s"))

    async def fake_exec(
        self: Any, command: str, *, timeout_secs: int | None = None, target: Any = None
    ) -> Any:
        del self, timeout_secs
        targets_seen.append(target)
        if command == "slow":
            raise ExecTimeoutError("IPC command timed out after 1s", command=command, status=504)
        if "slow504" in command:
            raise ExecTimeoutError("Gateway Timeout", command=command, status=504)
        if "slow408" in command:
            raise ExecTimeoutError("Request Timeout", command=command, status=408)
        if "false_positive_500" in command:
            raise HttpError(
                500,
                '{"code": "internal_error", "error": "database connection timed out in worker"}',
            )
        raise HttpError(500, "other failure")

    monkeypatch.setattr(capsem.Hypervisor, "managed", fake_managed)
    monkeypatch.setattr(capsem.VM, "exec", fake_exec)
    ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
    try:
        vid = await ctrl.start_vm(cpu_count=1, ram_gb=1)
        for cmd in ("slow", "slow504", "slow408"):
            with pytest.raises(TimeoutError, match="timed out"):
                await ctrl.exec_in_vm(vid, cmd, timeout=1)
        with pytest.raises(HttpError, match="database connection timed out"):
            await ctrl.exec_in_vm(vid, "false_positive_500", timeout=1)
        with pytest.raises(HttpError):
            await ctrl.exec_in_vm(vid, "fast", timeout=1)
        assert all(t == models.ExecTarget.VM for t in targets_seen)
    finally:
        await ctrl.close()


async def test_run_sandbox_command_and_user_formatting_edge_cases(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    ctrl = Scripted()
    assert (await run_sandbox_command(ctrl, "vm-1", [])).exit_code == 0
    ctrl.rules = [("timeout -k", fail(126, stderr="bash: Permission denied"))]
    with pytest.raises(PermissionError, match="Permission denied"):
        await run_sandbox_command(ctrl, "vm-1", ["tool"], timeout=5)

    ctrl.rules = []
    ctrl.commands.clear()
    res = await run_sandbox_command(
        ctrl, "vm-1", ["cat"], input_data="x" * 20000, env_vars={"A": "1"}, user="bob"
    )
    assert res.exit_code == 0 and any(".capsem_stdin_" in p for p in ctrl.uploads)
    assert (
        await run_sandbox_command(ctrl, "vm-1", ["cat"], input_data="short-in", cwd="sub")
    ).exit_code == 0
    await run_sandbox_command(ctrl, "vm-1", ["echo", "y" * 70000])
    assert any(".capsem_cmd_" in p for p in ctrl.uploads)

    assert is_root_user_spec("0:0") and not is_root_user_spec("0:1000")
    for spec, expected in (
        ("0:1000", ("setpriv --reuid=0 --regid=1000",)),
        ("1000", ("id -un 1000", "setpriv --reuid=1000 --regid=0")),
        ("1000:users", ("_uid=1000;", "getent group users")),
        ("alice:1000", ("_uid=$(id -u alice 2>/dev/null)", "_gid=1000;")),
        ("alice:users", ("_uid=$(id -u alice 2>/dev/null)", "getent group users")),
    ):
        cmd, _ = _format_exec_command("id", effective_cwd="/", env=None, user=spec, timeout=None)
        assert all(part in cmd for part in expected)
    for bad_env in ({"BAD-VAR": "1"}, {"123": "1"}):
        with pytest.raises(ValueError, match="Invalid environment variable name"):
            _format_exec_command("echo", effective_cwd="/", env=bad_env, user=None, timeout=None)

    uid = os.getuid()
    nonroot_script, _ = _format_exec_command(
        'printf "%s:%s" "$USER" "$LOGNAME"',
        effective_cwd="/",
        env=None,
        user=str(uid),
        timeout=None,
    )
    su_idx = nonroot_script.index("-s /bin/sh -c ") + len("-s /bin/sh -c ")
    su_body = shlex.split(nonroot_script[su_idx:])[0]
    sub_out = subprocess.run(
        ["/bin/sh", "-c", su_body], capture_output=True, text=True, check=True
    ).stdout
    expected_u = {f"{uid}:{uid}"}
    try:
        pw = pwd.getpwuid(uid)
        expected_u.add(f"{pw.pw_name}:{pw.pw_name}")
    except KeyError:
        pass
    assert sub_out in expected_u
    assert _wrap_target_command("id", is_container=True, user="0").startswith("sh -c ")

    class Slow(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            if "timeout -k" in command:
                raise TimeoutError(f"timed out after {timeout}s")
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    slow_ctrl = Slow()
    with pytest.raises(TimeoutError):
        await run_sandbox_command(
            slow_ctrl, "vm-1", ["sleep", "9"], input_data="x" * 20000, timeout=1
        )
    assert any(c.startswith("rm -f /tmp/.capsem_stdin_") for c in slow_ctrl.commands)

    shim_dir = tmp_path / "shim_bin"
    shim = str(write_portable_timeout_shim(shim_dir))
    assert (
        "GNU coreutils"
        in subprocess.run([shim, "--version"], capture_output=True, text=True).stdout
    )
    assert (
        subprocess.run([shim, "-k", "0.2s", "0.1s", "sh", "-c", "trap '' TERM; sleep 5"]).returncode
        == 124
    )
    for flag in (["--signal=KILL"], ["-s", "KILL"]):
        assert subprocess.run([shim, *flag, "0.1s", "sleep", "5"]).returncode == 137

    local_ctrl = LocalFakeCapsemController(tmp_path)
    res_kill = await run_sandbox_command(
        cast(Any, local_ctrl),
        "vm-sigkill",
        ["bash", "-c", "kill -KILL $$"],
        working_dir=str(tmp_path),
        timeout=30,
    )
    assert res_kill.exit_code == 137
    with pytest.raises(TimeoutError):
        await run_sandbox_command(
            cast(Any, local_ctrl),
            "vm-sigkill",
            ["bash", "-c", "trap '' TERM; sleep 5"],
            working_dir=str(tmp_path),
            timeout=1,
        )

    bin_dir = tmp_path / "minbin"
    bin_dir.mkdir()
    for tool in ("sh", "date", "sleep", "rm", "cat", "head", "mkdir", "mktemp", "ls"):
        for cand in (Path(f"/bin/{tool}"), Path(f"/usr/bin/{tool}")):
            if cand.exists():
                (bin_dir / tool).symlink_to(cand)
                break
    sh, min_env = str(bin_dir / "sh"), {"PATH": str(bin_dir)}
    script, _ = _format_exec_command(
        "sleep 5", effective_cwd=str(tmp_path), env=None, user=None, timeout=1
    )
    res_to = subprocess.run([sh, "-c", script], env=min_env, capture_output=True)
    assert res_to.returncode == 124 and exec_mod._TIMEOUT_SENTINEL.encode() in res_to.stderr
    (bin_dir / "mktemp").unlink()
    tok = os.urandom(6).hex()
    monkeypatch.setattr(exec_mod.uuid, "uuid4", lambda: type("U", (), {"hex": tok})())
    no_mkt, _ = _format_exec_command(
        "sleep 5", effective_cwd=str(tmp_path), env=None, user=None, timeout=1
    )
    try:
        res_to2 = subprocess.run([sh, "-c", no_mkt], env=min_env, capture_output=True)
        assert res_to2.returncode == 124 and exec_mod._TIMEOUT_SENTINEL.encode() in res_to2.stderr
    finally:
        for p in Path("/tmp").glob(f".capsem_to_{tok}_*"):
            shutil.rmtree(p, ignore_errors=True) if p.is_dir() else p.unlink(missing_ok=True)
    assert _host_timeout_skips() == set()
