"""Live Capsem VM and container conformance checks (`CAPSEM_LIVE_SELF_CHECK=1`)."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Literal

import pytest
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment

from .conftest import _run_inspect_self_check


@pytest.mark.skipif(
    not os.environ.get("CAPSEM_LIVE_SELF_CHECK"),
    reason="Set CAPSEM_LIVE_SELF_CHECK=1 with a running Capsem gateway/VM to run live self_check",
)
@pytest.mark.asyncio
async def test_live_capsem_vm_self_check() -> None:
    """Live conformance check against a real Capsem VM (vm and container modes)."""
    modes: tuple[Literal["vm", "container"], ...] = ("vm", "container")
    for mode in modes:
        envs = await CapsemSandboxEnvironment.sample_init(
            task_name=f"live_self_check_{mode}",
            config=CapsemSandboxConfig(execution_mode=mode, working_dir="/workspace"),
            metadata={},
        )
        try:
            sb = envs["default"]
            assert isinstance(sb, CapsemSandboxEnvironment)
            results = await _run_inspect_self_check(sb)
            failures = {k: v for k, v in results.items() if v is not True}
            print(f"live self_check {mode}: {len(results) - len(failures)}/{len(results)} passed")
            assert failures == {}, f"Live self_check ({mode}) failures: {failures}"
        finally:
            await CapsemSandboxEnvironment.sample_cleanup(
                task_name=f"live_self_check_{mode}",
                config=None,
                environments=envs,
                interrupted=False,
            )


@pytest.mark.skipif(
    not os.environ.get("CAPSEM_LIVE_SELF_CHECK"),
    reason="Set CAPSEM_LIVE_SELF_CHECK=1 with a running Capsem gateway/VM to run live non-root container check",
)
@pytest.mark.asyncio
async def test_live_container_nonroot_user_write_file_and_edit(tmp_path: Path) -> None:
    """Live container with non-root USER can exec and edit files staged via write_file."""
    df_path = tmp_path / "Dockerfile"
    df_path.write_text(
        "FROM python:3.11-slim\n"
        "RUN useradd -m -u 1000 -s /bin/bash developer && "
        "mkdir -p /workspace && chown developer:developer /workspace\n"
        "USER developer\n"
        "WORKDIR /workspace\n",
        encoding="utf-8",
    )
    envs = await CapsemSandboxEnvironment.sample_init(
        task_name="live_nonroot_write_file",
        config=CapsemSandboxConfig(
            execution_mode="container",
            dockerfile=str(df_path),
            working_dir="/workspace",
        ),
        metadata={},
    )
    try:
        sb = envs["default"]
        assert isinstance(sb, CapsemSandboxEnvironment)
        await sb.write_file("/workspace/staged.txt", "initial\n")
        stat_res = await sb.exec(["stat", "-c", "%u:%g", "/workspace/staged.txt"], user="root")
        assert stat_res.returncode == 0, stat_res.stderr
        assert stat_res.stdout.strip() == "1000:1000"
        dev_edit = await sb.exec(
            ["sh", "-c", 'test "$(id -u)" = 1000 && echo edited >> /workspace/staged.txt'],
            user="developer",
        )
        assert dev_edit.returncode == 0, dev_edit.stderr
        assert await sb.read_file("/workspace/staged.txt") == "initial\nedited\n"
        id_res = await sb.exec(["id", "-u"])
        assert id_res.returncode == 0, id_res.stderr
        assert id_res.stdout.strip() == "1000"
        un_res = await sb.exec(["id", "-un"])
        assert un_res.returncode == 0, un_res.stderr
        assert un_res.stdout.strip() == "developer"
        env_res = await sb.exec(
            ["sh", "-c", 'printf "%s:%s:%s" "$USER" "$LOGNAME" "$HOME"'],
            user="developer",
        )
        assert env_res.returncode == 0, env_res.stderr
        assert env_res.stdout == "developer:developer:/home/developer"
        default_env_res = await sb.exec(
            ["sh", "-c", 'printf "%s:%s:%s" "$USER" "$LOGNAME" "$HOME"']
        )
        assert default_env_res.returncode == 0, default_env_res.stderr
        assert default_env_res.stdout == "developer:developer:/home/developer"
        edit_res = await sb.exec(
            [
                "python3",
                "-c",
                "from pathlib import Path; p = Path('/workspace/staged.txt'); p.write_text(p.read_text() + 'edited2\\n')",
            ]
        )
        assert edit_res.returncode == 0, edit_res.stderr
        assert await sb.read_file("/workspace/staged.txt") == "initial\nedited\nedited2\n"
    finally:
        await CapsemSandboxEnvironment.sample_cleanup(
            task_name="live_nonroot_write_file", config=None, environments=envs, interrupted=False
        )
