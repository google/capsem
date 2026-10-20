"""Image catalog and prefetch through the service-owned image policy."""

from __future__ import annotations

from . import _operations as api
from . import models
from ._transport import Transport
from .registry import Registry


class Images:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def list(
        self, *, refresh: bool = False, request_timeout: float | None = None
    ) -> models.ImageListResponse:
        """Return catalog identity, compatible pins and the service's cache state."""
        return await api.list_images(
            self._transport, refresh=refresh, request_timeout=request_timeout
        )

    async def pull(
        self,
        image: str,
        *,
        registry: Registry | None = None,
        request_timeout: float | None = None,
    ) -> models.ImagePullResponse:
        """Prefetch an admitted image; registry access belongs to this call only."""
        if not isinstance(image, str) or not image.strip():
            raise ValueError("image must be a nonempty string")
        if registry is not None and not isinstance(registry, Registry):
            raise TypeError("registry must be a Registry object")
        request = models.ImagePullRequest(image=image)
        if registry is not None:
            request.registry = registry._wire()
        return await api.pull_image(
            self._transport, body=request, request_timeout=request_timeout
        )
