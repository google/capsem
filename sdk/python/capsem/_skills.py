"""Discover and read skill versions from the configured catalog."""
from __future__ import annotations

from . import _operations as api
from . import models
from ._transport import Transport


class Skills:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def list(
        self, *, search: str | None = None, after: str | None = None,
        limit: int | None = None, request_timeout: float | None = None,
    ) -> models.SkillCatalogListResponse:
        """List published skill versions and the snapshot to use for detail reads."""
        return await api.list_catalog_skills(
            self._transport, search=search, after=after, limit=limit, request_timeout=request_timeout,
        )

    async def get(
        self, skill_id: str, version: str, catalog: models.SkillCatalogSnapshot, *,
        request_timeout: float | None = None,
    ) -> models.SkillCatalogDetailResponse:
        """Read the complete file dictionary for the chosen version and snapshot."""
        result = await api.get_catalog_skill(
            self._transport, skill_id=skill_id, version=version, snapshot=catalog.snapshot,
            request_timeout=request_timeout,
        )
        if (
            result.selection.id != skill_id or result.selection.version != version
            or result.metadata.id != skill_id or result.metadata.version != version
            or result.selection.catalog_reference != catalog.reference
            or result.selection.resolved_digest != catalog.digest
        ):
            raise ValueError("catalog skill identity changed")
        return result
