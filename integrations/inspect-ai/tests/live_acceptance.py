"""Live Capsem VM and OCI container acceptance checks for `inspect-capsem-sandbox`."""

from __future__ import annotations

import asyncio
import functools
import os
import sqlite3
import tempfile
from pathlib import Path
from typing import Any, cast

import inspect_ai.util._sandbox.self_check as inspect_self_check
import inspect_capsem.sandbox as sb_mod
from capsem import Hypervisor, models
from inspect_ai import Task, eval_async
from inspect_ai.dataset import Sample
from inspect_ai.scorer import includes
from inspect_ai.solver import Generate, Solver, TaskState, solver
from inspect_ai.util import SandboxEnvironmentSpec, sandbox
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment
from inspect_capsem._controller import (
    SdkCapsemController,
    _is_managed_vm,
    _managed_vm_labels,
)

try:
    from tests.oci_workload_fixture import hermetic_oci_workload_image
except ImportError:
    import importlib.util

    _spec = importlib.util.spec_from_file_location(
        "oci_workload_fixture",
        Path(__file__).resolve().with_name("oci_workload_fixture.py"),
    )
    assert _spec is not None and _spec.loader is not None
    _mod = importlib.util.module_from_spec(_spec)
    _spec.loader.exec_module(_mod)
    hermetic_oci_workload_image = _mod.hermetic_oci_workload_image


def _capsem_home_dir() -> Path:
    raw = os.environ.get("CAPSEM_HOME", "").strip()
    return Path(raw) if raw else Path.home() / ".capsem"


async def _verify_session_ledger(
    hyp: Hypervisor, vm_id: str, *, expected_target: str, marker: str
) -> None:
    history = await hyp.vm(id=vm_id).history(layer=models.HistoryLayerFilter.EXEC, limit=50)
    assert history.total >= 1
    assert any(marker in entry.command for entry in history.commands)

    home = _capsem_home_dir()
    candidates = [
        home / "run" / "sessions" / vm_id / "session.db",
        home / "sessions" / vm_id / "session.db",
    ]
    db_path = next((p for p in candidates if p.is_file()), candidates[0])
    assert db_path.is_file(), f"expected session ledger at {db_path}"
    with sqlite3.connect(f"file:{db_path}?mode=ro", uri=True) as conn:
        rows = conn.execute(
            "SELECT target, command, exit_code FROM exec_events WHERE command LIKE ?",
            (f"%{marker}%",),
        ).fetchall()
    assert any(target == expected_target and exit_code == 0 for target, _cmd, exit_code in rows), (
        f"expected {expected_target} exec_events row for {marker}, got {rows}"
    )


async def _run_inspect_self_check(
    env: CapsemSandboxEnvironment, *, skip: set[str] | None = None
) -> dict[str, bool | str]:
    results: dict[str, bool | str] = {}
    for name in inspect_self_check.__all__:
        if skip and name in skip:
            continue
        fn = getattr(inspect_self_check, name)
        try:
            await fn(env)
            results[name] = True
        except Exception as exc:
            results[name] = f"{type(exc).__name__}: {exc}"
    return results


@solver
def _sandbox_probe_solver(marker: str) -> Solver:
    async def solve(state: TaskState, generate: Generate) -> TaskState:
        del generate
        res = await sandbox().exec(["sh", "-c", f"echo {marker}"])
        assert res.returncode == 0, res.stderr
        state.output.completion = res.stdout.strip()
        return state

    return solve


async def _verify_eval_task(hyp: Hypervisor, config: CapsemSandboxConfig, marker: str) -> None:
    with tempfile.TemporaryDirectory(prefix="inspect-capsem-eval-") as log_dir:
        task = Task(
            dataset=[Sample(input="probe", target=marker)],
            solver=_sandbox_probe_solver(marker),
            scorer=includes(),
            sandbox=SandboxEnvironmentSpec("capsem", config),
        )
        logs = await eval_async(task, model="mockllm/model", log_dir=log_dir)
        assert len(logs) == 1 and logs[0].status == "success", logs
        assert logs[0].samples and logs[0].samples[0].output.completion == marker
    leaked = [m.id for m in (await hyp.list()).sandboxes if _is_managed_vm(m)]
    assert not leaked, f"eval_async leaked ephemeral managed VMs: {leaked}"


async def _verify_container_workload_mode(hyp: Hypervisor, image_ref: str, ca_pem: str) -> None:
    orig_ctrl = sb_mod.SdkCapsemController
    cast(Any, sb_mod).SdkCapsemController = functools.partial(
        SdkCapsemController, registry_ca_pem=ca_pem
    )
    try:
        envs = await CapsemSandboxEnvironment.sample_init(
            task_name="gate_container_acceptance",
            config=CapsemSandboxConfig(
                image=image_ref, cpu_count=2, ram_gb=2, working_dir="/workspace"
            ),
            metadata={},
        )
        vm_id = ""
        try:
            sb_env = envs["default"]
            assert isinstance(sb_env, CapsemSandboxEnvironment)
            sb, vm_id = sb_env, sb_env.vm_id
            marker_res = await sb.exec(["cat", "/etc/capsem-workload-fixture"])
            assert marker_res.returncode == 0 and (
                marker_res.stdout.strip() == "capsem-hermetic-oci-fixture"
            )
            exec_res = await sb.exec(["echo", "INSPECT_CAPSEM_CONTAINER_ACCEPTANCE_OK"])
            assert exec_res.returncode == 0 and (
                exec_res.stdout.strip() == "INSPECT_CAPSEM_CONTAINER_ACCEPTANCE_OK"
            )
            await sb.write_file("/workspace/container_roundtrip.txt", "container-ok\n")
            assert await sb.read_file("/workspace/container_roundtrip.txt") == "container-ok\n"
            payload = bytes(range(32)) + b"\x00CONTAINER_BIN\xff"
            await sb.write_file("/workspace/container.bin", payload)
            assert await sb.read_file("/workspace/container.bin", text=False) == payload
            await _verify_session_ledger(
                hyp,
                sb.vm_id,
                expected_target="workload",
                marker="INSPECT_CAPSEM_CONTAINER_ACCEPTANCE_OK",
            )
        finally:
            await CapsemSandboxEnvironment.sample_cleanup(
                task_name="gate_container_acceptance",
                config=None,
                environments=envs,
                interrupted=False,
            )
            await CapsemSandboxEnvironment.task_cleanup(
                task_name="gate_container_acceptance", config=None, cleanup=True
            )
        assert vm_id not in {m.id for m in (await hyp.list()).sandboxes}, (
            f"container sample_cleanup leaked VM {vm_id}"
        )
        await _verify_eval_task(
            hyp,
            CapsemSandboxConfig(image=image_ref, cpu_count=1, ram_gb=1, working_dir="/workspace"),
            "INSPECT_CAPSEM_CONTAINER_EVAL_OK",
        )
        print("INSPECT_CAPSEM_CONTAINER_ACCEPTANCE_OK")
    finally:
        cast(Any, sb_mod).SdkCapsemController = orig_ctrl


async def run_live_vm_sandbox_acceptance() -> None:
    """VM and hermetic OCI container acceptance test for the gate VM lane."""
    hyp = Hypervisor.connect()
    persistent_vm = await hyp.create(
        name=f"foreign-persistent-{os.getpid()}",
        cpus=1,
        memory=1,
        labels=_managed_vm_labels(task_name="gate_vm_acceptance"),
    )
    orphan_ctrl = SdkCapsemController(hypervisor=hyp)
    orphan_id: str | None = None
    try:
        envs = await CapsemSandboxEnvironment.sample_init(
            task_name="gate_vm_acceptance",
            config=CapsemSandboxConfig(
                cpu_count=2,
                ram_gb=2,
                working_dir="/workspace",
            ),
            metadata={},
        )
        vm_id = ""
        try:
            sb_env = envs["default"]
            assert isinstance(sb_env, CapsemSandboxEnvironment)
            sb = sb_env
            vm_id = sb.vm_id
            exec_res = await sb.exec(["echo", "INSPECT_CAPSEM_VM_ACCEPTANCE_OK"])
            assert exec_res.returncode == 0, exec_res.stderr
            assert exec_res.stdout.strip() == "INSPECT_CAPSEM_VM_ACCEPTANCE_OK"
            nz_res = await sb.exec(["sh", "-c", "exit 7"])
            assert (nz_res.success, nz_res.returncode) == (False, 7)
            kill_res = await sb.exec(["bash", "-c", "kill -KILL $$"], timeout=10)
            assert (kill_res.success, kill_res.returncode) == (False, 137)
            try:
                await sb.exec(["sleep", "5"], timeout=1)
            except TimeoutError:
                pass
            else:
                raise AssertionError("expected TimeoutError on live VM sleep")
            await sb.write_file("/workspace/roundtrip.txt", "hello-capsem\n")
            assert await sb.read_file("/workspace/roundtrip.txt") == "hello-capsem\n"
            await sb.write_file("/var/tmp/non_ws_roundtrip.txt", "non-ws-ok\n")
            assert await sb.read_file("/var/tmp/non_ws_roundtrip.txt") == "non-ws-ok\n"
            payload = bytes(range(64)) + b"\x00CAPSEM_BIN\xff"
            await sb.write_file("/workspace/roundtrip.bin", payload)
            assert await sb.read_file("/workspace/roundtrip.bin", text=False) == payload
            await _verify_session_ledger(
                hyp,
                sb.vm_id,
                expected_target="vm",
                marker="INSPECT_CAPSEM_VM_ACCEPTANCE_OK",
            )
            # Skip test_read_and_write_large_file_binary (>100 MiB allocation/transfer) in live
            # gate runs; Capsem's regular-file byte cap and OutputLimitExceededError path are
            # covered deterministically by unit tests in test_files.py.
            self_check_results = await _run_inspect_self_check(
                sb, skip={"test_read_and_write_large_file_binary"}
            )
            failures = {k: v for k, v in self_check_results.items() if v is not True}
            assert not failures, f"Live self_check (vm) failures: {failures}"
            await sb._controller.stop_vm("nonexistent-vm-404")
        finally:
            await CapsemSandboxEnvironment.sample_cleanup(
                task_name="gate_vm_acceptance",
                config=None,
                environments=envs,
                interrupted=False,
            )
            await CapsemSandboxEnvironment.task_cleanup(
                task_name="gate_vm_acceptance",
                config=None,
                cleanup=True,
            )
        assert vm_id not in {m.id for m in (await hyp.list()).sandboxes}, (
            f"sample_cleanup leaked VM {vm_id}"
        )

        await _verify_eval_task(
            hyp,
            CapsemSandboxConfig(cpu_count=1, ram_gb=1, working_dir="/workspace"),
            "INSPECT_CAPSEM_VM_EVAL_OK",
        )

        with hermetic_oci_workload_image(_capsem_home_dir()) as (image_ref, ca_pem):
            await _verify_container_workload_mode(hyp, image_ref, ca_pem)

        orphan_id = await orphan_ctrl.start_vm(cpu_count=1, ram_gb=1)
        by_id = {m.id: m for m in (await hyp.list()).sandboxes}
        assert persistent_vm.id in by_id and by_id[persistent_vm.id].persistent is True
        assert orphan_id in by_id and by_id[orphan_id].persistent is False
        assert by_id[orphan_id].name != orphan_id

        await CapsemSandboxEnvironment.cli_cleanup(None)
        remaining_ids = {m.id for m in (await hyp.list()).sandboxes}
        assert orphan_id not in remaining_ids, f"orphaned ephemeral VM {orphan_id} was not swept"
        orphan_id = None
        assert persistent_vm.id in remaining_ids, "persistent=True VM did not survive cli_cleanup"
        print("INSPECT_CAPSEM_VM_ACCEPTANCE_OK")
    finally:
        if orphan_id is not None:
            await orphan_ctrl.stop_vm(orphan_id)
        await persistent_vm.delete()
        leaked_final = [m.id for m in (await hyp.list()).sandboxes if _is_managed_vm(m)]
        assert not leaked_final, f"leaked managed VMs at teardown: {leaked_final}"
        await hyp.close()


async def main() -> None:
    await run_live_vm_sandbox_acceptance()


if __name__ == "__main__":
    asyncio.run(main())
