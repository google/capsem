"""Run the installed SDK against an isolated real service/gateway fixture."""

from __future__ import annotations

import asyncio
import os

from capsem import VM, HttpError, Hypervisor, models


async def main() -> None:
    url, token = os.environ["SDK_GATEWAY_URL"], os.environ["SDK_GATEWAY_TOKEN"]
    expected_id = os.environ["SDK_VM_ID"]
    async with Hypervisor(url, "incorrect-token") as denied:
        try:
            await denied.list()
        except HttpError as error:
            assert error.status == 401
        else:
            raise AssertionError("gateway accepted incorrect SDK credentials")
    async with Hypervisor(url, token) as hv, VM(url, token, name="route-workspace") as vm:
        assert isinstance(await hv.info(), models.HypervisorInfo)
        inventory = await hv.list()
        assert any(entry.id == expected_id for entry in inventory.sandboxes)
        files = await vm.list("/")
        assert vm.id == expected_id
        assert {entry.name for entry in files.entries} >= {"modified.txt", "created.txt"}
        snapshots = await vm.snapshots.list()
        assert snapshots.total == 1 and snapshots.snapshots[0].checkpoint == "cp-10"
        changes = await vm.changes("cp-10")
        assert {(entry.path, entry.kind) for entry in changes.changes} == {
            ("created.txt", models.FileChangeKind.CREATED),
            ("modified.txt", models.FileChangeKind.MODIFIED),
            ("deleted.txt", models.FileChangeKind.DELETED),
        }
        for call in (lambda: vm.copy.from_vm("/created.txt"), lambda: vm.copy.to_vm("/refused.txt", b"new")):
            try:
                await call()
            except HttpError as error:
                assert error.status == 409 and "running sandbox security ledger" in error.body
            else:
                raise AssertionError("stopped copy bypassed the security ledger")
        assert "refused.txt" not in {entry.name for entry in (await vm.list()).entries}
    async with VM(url, token, id=expected_id) as vm:
        assert (await vm.snapshots.status()).total == 1
    print("SDK_GATEWAY_ACCEPTANCE_OK")


if __name__ == "__main__":
    asyncio.run(main())
