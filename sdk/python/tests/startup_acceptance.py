"""Installed SDK creation outlives its former wait budget and finite lease."""
from __future__ import annotations

import asyncio
import os
import time
import uuid

from capsem import Hypervisor, models


async def main() -> None:
    selected = models.EnvironmentRevisionReference(
        environment_id=os.environ["SDK_ENVIRONMENT_ID"], revision_id=os.environ["SDK_REVISION_ID"])
    async with Hypervisor(os.environ["SDK_GATEWAY_URL"], os.environ["SDK_GATEWAY_TOKEN"]) as hv:
        started = time.monotonic()
        scope = hv.managed(environment=selected, cpus=4, memory=12)
        async with scope as vm:
            assert time.monotonic() - started >= 235, "creation answered before approved startup completed"
            assert scope.ownership is not None and scope.ownership.lease_seconds == 30
            result = await vm.exec('test "$(cat /workspace/long-startup)" = retained; redis-cli PING')
            assert result.exit_code == 0 and result.stdout_bytes == b"PONG\n" and result.stderr_bytes == b""
            info = await vm.info()
            assert info.environment is not None and info.environment.boot == selected
            assert (await vm.container.status()).state == models.ContainerState.RUNNING
            original = vm.id
        assert not any(row.id == original for row in (await hv.list()).sandboxes)

        tag = str(uuid.uuid4())

        async def preparing() -> None:
            async with hv.managed(environment=selected, cpus=4, memory=12, labels={"probe": tag}):
                raise AssertionError("cancelled startup reached its entrypoint")

        pending = asyncio.create_task(preparing())
        preparing_id = None
        try:
            deadline = time.monotonic() + 45
            while time.monotonic() < deadline:
                if pending.done():
                    await pending
                for row in (await hv.list()).sandboxes:
                    if (row.labels or {}).get("probe") != tag:
                        continue
                    status = await hv.vm(id=row.id).container.status()
                    if status.state == models.ContainerState.PREPARING:
                        preparing_id = row.id
                        break
                if preparing_id is not None:
                    break
                await asyncio.sleep(0.05)
            assert preparing_id is not None, "no real preparing workload observed"
            pending.cancel()
            try:
                await pending
            except asyncio.CancelledError:
                pass
            else:
                raise AssertionError("startup cancellation was swallowed")
            assert not any(row.id == preparing_id for row in (await hv.list()).sandboxes)
        finally:
            if not pending.done():
                pending.cancel()
            await asyncio.gather(pending, return_exceptions=True)
        assert (await hv.list()).sandboxes == []
        print("SDK_STARTUP_DEADLINE_OK lease=renewed preparing=cancelled cleanup=complete")


if __name__ == "__main__":
    asyncio.run(main())
