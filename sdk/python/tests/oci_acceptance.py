"""Run the installed SDK against an admitted image in a hermetic TLS registry."""

from __future__ import annotations

import asyncio
import os
import uuid
from pathlib import Path

from capsem import VM, Hypervisor, Registry, models

WORKLOAD = (
    "printf SDK_OCI_WORKLOAD; test -x /usr/local/bin/redis-cli; "
    "test ! -e /var/tmp/capsem-container/workload.pid"
)
GUEST = "printf SDK_OCI_VM; test -s /var/tmp/capsem-container/workload.pid"


async def entered(vm: VM) -> None:
    # This is the very first operation after create/start/resume. Workload
    # readiness cannot be manufactured by polling before the exec.
    result = await vm.exec(WORKLOAD)
    assert result.exit_code == 0 and result.stdout.data == "SDK_OCI_WORKLOAD", (
        result.exit_code, result.stdout.data, result.stderr_bytes,
    )
    assert result.stderr_bytes == b""
    guest = await vm.exec(GUEST, target=models.ExecTarget.VM)
    assert guest.exit_code == 0 and guest.stdout.data == "SDK_OCI_VM", guest
    assert guest.stderr_bytes == b""


async def main() -> None:
    reference = os.environ["SDK_IMAGE"]
    ca = Path(os.environ["SDK_REGISTRY_CA"]).read_text()
    name = "sdk-oci-" + uuid.uuid4().hex[:8]
    async with Hypervisor(os.environ["SDK_GATEWAY_URL"], os.environ["SDK_GATEWAY_TOKEN"]) as hv:
        for selected_name in (name, ""):
            vm = await hv.create(name=selected_name, cpus=2, memory=2, image=reference,
                                 registry=Registry(ca_pem=ca))
            await entered(vm)
            info = await vm.info()
            assert info.id == vm.id and info.persistent == bool(selected_name)
            assert info.status == models.VmLifecycleState.RUNNING
            status = await vm.container.status()
            assert isinstance(status, models.ContainerStatusResponse)
            assert status.state == models.ContainerState.RUNNING and status.error is None
            assert status.image == reference and status.resolved == reference
            assert status.digest == reference.split("@", 1)[1]
            if selected_name:
                stopped = await vm.stop()
                assert stopped.success and stopped.persistent
                assert (await vm.info()).status == models.VmLifecycleState.STOPPED
                await vm.start()
                await entered(vm)
                await vm.pause()
                assert (await vm.info()).status == models.VmLifecycleState.SUSPENDED
                await vm.resume()
                await entered(vm)
                assert (await vm.delete()).success
            else:
                stopped = await vm.stop()
                assert stopped.success and not stopped.persistent
            assert not any(entry.id == vm.id for entry in (await hv.list()).sandboxes)
        assert not (await hv.list()).sandboxes
    print("SDK_OCI_ACCEPTANCE_OK named=1 unnamed=1 workload=entered vm=entered")


if __name__ == "__main__":
    asyncio.run(main())
