"""Exercise public SDK lifecycle and byte transfer against a disposable real VM."""

from __future__ import annotations

import asyncio
import gzip
import hashlib
import json
import os
import uuid

from capsem import VM, HttpError, Hypervisor, models


async def _assert_live_exports(vm: VM, vm_id: str) -> None:
    rows_raw = gzip.decompress(await vm.export.rows()).decode()
    lines = [json.loads(line) for line in rows_raw.splitlines() if line.strip()]
    assert lines and lines[0]["type"] == "header" and lines[0]["format"] == "capsem.rows.v1"
    assert lines[0]["format_version"] == 1 and lines[0]["vm_id"] == vm_id
    assert any(item.get("table") == "exec_events" for item in lines[1:])
    warc_raw = gzip.decompress(await vm.export.warc())
    assert warc_raw.startswith(b"WARC/1.1\r\n")


async def ready(vm: VM) -> None:
    async with asyncio.timeout(60):
        while True:
            result = await vm.exec("printf SDK_EXEC_READY", timeout_secs=5)
            if result.exit_code == 0 and result.stdout.data == "SDK_EXEC_READY":
                return
            await asyncio.sleep(0.2)


async def main() -> None:
    async with Hypervisor(os.environ["SDK_GATEWAY_URL"], os.environ["SDK_GATEWAY_TOKEN"], timeout=120) as hv:
        name = "sdk-live-" + uuid.uuid4().hex[:8]
        vm = await hv.create(name=name, cpus=2, memory=2)
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
            uploaded = await vm.files.write("sdk-proof.bin", data)
            assert uploaded.success and uploaded.size == len(data)
            assert await vm.files.read("sdk-proof.bin") == data
            assert "sdk-proof.bin" in {entry.name for entry in (await vm.files.list()).entries}
            digest = hashlib.sha256(data).hexdigest()
            executed = await vm.exec("sha256sum /root/sdk-proof.bin; printf SDK_STDERR >&2; exit 7")
            assert executed.exit_code == 7 and executed.stdout.data.split()[0] == digest
            assert executed.stderr.data == "SDK_STDERR"
            assert vm.id is not None
            await _assert_live_exports(vm, vm.id)
            assert isinstance(await vm.log(tail=10), models.LogsResponse)
            assert isinstance(await hv.log(tail=10), models.HostLogsResponse)
            assert isinstance(await vm.history(limit=10), models.HistoryResponse)
            assert isinstance(await vm.stats.summary(), models.VmStatsSummaryResponse)
            assert isinstance(await vm.stats.details(), models.VmStatsDetailResponse)
            stopped = await vm.stop()
            assert stopped.success and stopped.persistent
            assert (await vm.info()).status == models.VmLifecycleState.STOPPED
            fork = await vm.fork(name + "-fork")
            assert fork.id != vm.id and (await fork.info()).forked_from == vm.id
            await fork.start()
            await ready(fork)
            assert await fork.files.read("sdk-proof.bin") == data
            await fork.files.write("sdk-proof.bin", b"fork-only")
            await vm.start()
            await ready(vm)
            assert await vm.files.read("sdk-proof.bin") == data
            await vm.pause()
            assert (await vm.info()).status == models.VmLifecycleState.SUSPENDED
            await vm.resume()
            await ready(vm)
            assert await vm.files.read("sdk-proof.bin") == data
            completed = True
        finally:
            # On failure the owning service fixture preserves evidence before
            # deleting its VMs; deleting here would erase the boot failure log.
            if completed:
                if fork is not None:
                    await fork.delete()
                await vm.delete()
        assert not any(entry.name in {name, name + "-fork"} for entry in (await hv.list()).sandboxes)
        async with hv.managed(cpus=2, memory=2) as mvm:
            await ready(mvm)
            assert mvm.id is not None
            managed_id = mvm.id
            stopped_managed = await mvm.stop()
            assert stopped_managed.success and not stopped_managed.persistent
            await _assert_live_exports(mvm, managed_id)
        closed_vm = hv.vm(id=managed_id)
        for export_coro in (closed_vm.export.rows(), closed_vm.export.warc()):
            try:
                await export_coro
            except HttpError as err:
                assert err.status == 404
            else:
                raise AssertionError("expected 404 after managed session close")
        print(f"SDK_LIVE_ACCEPTANCE_OK bytes={len(data)} sha256={digest}")


if __name__ == "__main__":
    asyncio.run(main())
