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
        profiles = await hv.profiles.list()
        assert profiles
        mcp = hv.profiles.mcp(profiles[0])
        assert (await mcp.info()).profile_id == profiles[0].id
        assert isinstance(await hv.debug.panics(limit=2), models.PanicsResponse)
        assert isinstance(await hv.debug.triage(since="1h", limit=2), models.TriageResponse)
        inventory = await hv.list()
        assert any(entry.id == expected_id for entry in inventory.sandboxes)
        files = await vm.files.list()
        assert vm.id == expected_id
        assert {entry.name for entry in files.entries} >= {"modified.txt", "created.txt"}
        for call in (lambda: vm.files.read("/root/created.txt"), lambda: vm.files.write("/root/refused.txt", b"new")):
            try:
                await call()
            except HttpError as error:
                assert error.status == 409 and "running sandbox security ledger" in error.body
            else:
                raise AssertionError("stopped copy bypassed the security ledger")
        assert "refused.txt" not in {entry.name for entry in (await vm.files.list()).entries}
    async with VM(url, token, id=expected_id) as vm:
        assert vm.id == expected_id
    print("BRAAVOS_SDK_ACCEPTANCE_OK")


if __name__ == "__main__":
    asyncio.run(main())
