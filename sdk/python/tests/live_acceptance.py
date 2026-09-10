"""Exercise public SDK lifecycle and byte transfer against a disposable real VM."""

from __future__ import annotations

import asyncio
import hashlib
import os
import uuid

from capsem import VM, Hypervisor, models


async def ready(vm: VM) -> None:
    async with asyncio.timeout(60):
        while True:
            result = await vm.exec("printf SDK_EXEC_READY", timeout_secs=5)
            if result.exit_code == 0 and result.stdout == "SDK_EXEC_READY":
                return
            await asyncio.sleep(0.2)


async def main() -> None:
    async with Hypervisor(os.environ["SDK_GATEWAY_URL"], os.environ["SDK_GATEWAY_TOKEN"], timeout=120) as hv:
        name = "sdk-live-" + uuid.uuid4().hex[:8]
        vm = await hv.create(os.environ.get("CAPSEM_TEST_PROFILE", "code"), name=name, vcpu=2, memory="2G")
        fork: VM | None = None
        completed = False
        try:
            await ready(vm)
            info = await vm.info()
            assert info.id == vm.id and info.name == name and info.persistent
            assert info.status == models.VmLifecycleState.RUNNING
            assert info.ai is not None and info.network is not None and info.files is not None
            assert info.cpus == 2 and info.ram_mb == 2048
            data = bytes(range(256)) * 17 + b"\x00SDK_BINARY\xff"
            uploaded = await vm.copy.to_vm("sdk-proof.bin", data)
            assert uploaded.success and uploaded.size == len(data)
            assert await vm.copy.from_vm("sdk-proof.bin") == data
            assert "sdk-proof.bin" in {entry.name for entry in (await vm.list()).entries}
            digest = hashlib.sha256(data).hexdigest()
            executed = await vm.exec("sha256sum /root/sdk-proof.bin; printf SDK_STDERR >&2; exit 7")
            assert executed.exit_code == 7 and executed.stdout.split()[0] == digest
            # The guest exec channel currently combines stdout and stderr.
            assert executed.stdout.endswith("\nSDK_STDERR") and executed.stderr == ""
            assert isinstance(await vm.log(tail=10), models.LogsResponse)
            assert isinstance(await hv.log(tail=10), models.HostLogsResponse)
            assert isinstance(await vm.history(limit=10), models.HistoryResponse)
            assert isinstance(await vm.stats.summary(), models.VmStatsSummaryResponse)
            assert isinstance(await vm.stats.details(), models.VmStatsDetailResponse)
            assert isinstance(await vm.snapshots.list(), models.SnapshotsList)
            assert isinstance(await vm.snapshots.status(), models.SnapshotsStatus)
            stopped = await vm.stop()
            assert stopped.success and stopped.persistent
            assert (await vm.info()).status == models.VmLifecycleState.STOPPED
            fork = await vm.fork(name + "-fork")
            assert fork.id != vm.id and (await fork.info()).forked_from == vm.id
            await fork.start()
            await ready(fork)
            assert await fork.copy.from_vm("sdk-proof.bin") == data
            await fork.copy.to_vm("sdk-proof.bin", b"fork-only")
            await vm.start()
            await ready(vm)
            assert await vm.copy.from_vm("sdk-proof.bin") == data
            await vm.pause()
            assert (await vm.info()).status == models.VmLifecycleState.SUSPENDED
            await vm.resume()
            await ready(vm)
            assert await vm.copy.from_vm("sdk-proof.bin") == data
            print(f"SDK_LIVE_ACCEPTANCE_OK bytes={len(data)} sha256={digest}")
            completed = True
        finally:
            # On failure the owning service fixture preserves evidence before
            # deleting its VMs; deleting here would erase the boot failure log.
            if completed:
                if fork is not None:
                    await fork.delete()
                await vm.delete()
        assert not any(entry.name in {name, name + "-fork"} for entry in (await hv.list()).sandboxes)


if __name__ == "__main__":
    asyncio.run(main())
