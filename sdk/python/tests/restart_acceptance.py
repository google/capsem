"""A managed restart is sent once through the public Python facade."""

import asyncio
import os

from capsem import Hypervisor, models


async def main() -> None:
    async with Hypervisor(os.environ["SDK_GATEWAY_URL"], os.environ["SDK_GATEWAY_TOKEN"]) as hv:
        response = await hv.restart()
        assert response.status is models.RestartStatus.ACCEPTED
        assert response.authentication is models.RestartAuthentication.NEW_TOKEN_REQUIRED
        assert response.manager is models.ServiceManager.LAUNCHD
    print("SDK_RESTART_ACCEPTED")


if __name__ == "__main__":
    asyncio.run(main())
