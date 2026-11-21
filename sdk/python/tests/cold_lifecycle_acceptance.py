"""Exercise inherited startup through a cleanly installed SDK."""

from __future__ import annotations

import asyncio
import os
import time
import uuid

from capsem import Hypervisor, models


async def main() -> None:
    reference = models.EnvironmentRevisionReference(
        environment_id=os.environ["SDK_ENVIRONMENT_ID"],
        revision_id=os.environ["SDK_REVISION_ID"],
    )
    start = time.monotonic()
    async with Hypervisor(os.environ["SDK_GATEWAY_URL"], os.environ["SDK_GATEWAY_TOKEN"]) as hv:
        vm = await hv.create(
            from_vm=os.environ["SDK_COLD_SOURCE_NAME"],
            name=f"cold-package-{uuid.uuid4()}",
            memory=int(os.environ["SDK_COLD_MEMORY_GIB"]),
            cpus=int(os.environ["SDK_COLD_CPUS"]),
        )
        try:
            assert vm.id != os.environ["SDK_VM_ID"]
            assert time.monotonic() - start >= int(os.environ["SDK_COLD_MIN_SECONDS"])
            info = await vm.info()
            assert info.environment is not None and info.environment.boot == reference
            assert info.environment.owner_generation != os.environ["SDK_COLD_SOURCE_GENERATION"]
            assert (await vm.container.status()).state is models.ContainerState.RUNNING
            result = await vm.exec(
                'test "$(cat /workspace/entrypoint-count)" = 2 && '
                'test "$BOUND_REVISION" = retained-variable && redis-cli PING'
            )
            assert result.exit_code == 0 and result.stdout_bytes == b"PONG\n"
        finally:
            try:
                await vm.delete()
            finally:
                await vm.close()
    print("SDK_COLD_LIFECYCLE_OK")


if __name__ == "__main__":
    asyncio.run(main())
