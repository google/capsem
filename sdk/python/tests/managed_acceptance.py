"""An installed SDK scope awaits native VM cleanup through the real gateway."""

from __future__ import annotations

import asyncio
import os

from capsem import Hypervisor


async def main() -> None:
    async with Hypervisor(os.environ["SDK_GATEWAY_URL"], os.environ["SDK_GATEWAY_TOKEN"], timeout=120) as hv:
        scope = hv.managed(cpus=4, memory=12)
        async with scope as vm:
            info = await vm.info()
            assert info.id == vm.id and not info.persistent
            assert info.cpus == 4 and info.ram_mb == 12288
            ownership = scope.ownership
            assert ownership is not None
            assert ownership.lease_seconds == 300 and ownership.renew_after_seconds == 60
            result = await vm.exec("printf MANAGED_SDK_NATIVE_READY", timeout_secs=60)
            assert result.exit_code == 0 and result.stdout.data == "MANAGED_SDK_NATIVE_READY"
            session_id = vm.id
        assert not any(row.id == session_id for row in (await hv.list()).sandboxes)
        await scope.close()
        assert not any(row.id == session_id for row in (await hv.list()).sandboxes)
        print("SDK_MANAGED_ACCEPTANCE_OK")


if __name__ == "__main__":
    asyncio.run(main())
