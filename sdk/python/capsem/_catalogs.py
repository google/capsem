"""Discover exact published versions from the configured catalogs."""
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
        self, skill_id: str, version: str, catalog: models.CatalogSnapshot, *,
        request_timeout: float | None = None,
    ) -> models.SkillCatalogDetailResponse:
        """Read the complete file dictionary for the chosen version and snapshot."""
        result = await api.get_catalog_skill(
            self._transport, skill_id=skill_id, version=version, snapshot=catalog.snapshot,
            request_timeout=request_timeout,
        )
        _match_entry(skill_id, version, catalog, result.selection, result.metadata)
        return result


class Rules:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def list(
        self, *, search: str | None = None, after: str | None = None,
        limit: int | None = None, request_timeout: float | None = None,
    ) -> models.RuleCatalogListResponse:
        """List the published access and detection rules in the configured catalog."""
        return await api.list_catalog_rules(
            self._transport, search=search, after=after, limit=limit, request_timeout=request_timeout,
        )

    async def get(
        self, rule_id: str, version: str, catalog: models.CatalogSnapshot, *,
        request_timeout: float | None = None,
    ) -> models.RuleCatalogDetailResponse:
        """Read the typed definition and parameters of a listed rule version."""
        result = await api.get_catalog_rule(
            self._transport, rule_id=rule_id, version=version, snapshot=catalog.snapshot,
            request_timeout=request_timeout,
        )
        _match_entry(rule_id, version, catalog, result.selection, result.metadata)
        return result


class Mcps:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def list(
        self, *, search: str | None = None, after: str | None = None,
        limit: int | None = None, request_timeout: float | None = None,
    ) -> models.McpCatalogListResponse:
        """List published server templates from the configured catalog."""
        return await api.list_catalog_mcps(
            self._transport, search=search, after=after, limit=limit, request_timeout=request_timeout,
        )

    async def get(
        self, mcp_id: str, version: str, catalog: models.CatalogSnapshot, *,
        request_timeout: float | None = None,
    ) -> models.McpCatalogDetailResponse:
        """Read published configuration requirements and tool metadata for one version."""
        result = await api.get_catalog_mcp(
            self._transport, mcp_id=mcp_id, version=version, snapshot=catalog.snapshot,
            request_timeout=request_timeout,
        )
        _match_entry(mcp_id, version, catalog, result.selection, result.metadata)
        return result


def _match_entry(
    entry_id: str, version: str, catalog: models.CatalogSnapshot,
    selection: models.EnvironmentCatalogPin, metadata: models.CatalogEntryMetadata,
) -> None:
    if (
        selection.id != entry_id or selection.version != version
        or metadata.id != entry_id or metadata.version != version
        or selection.catalog_reference != catalog.reference
        or selection.resolved_digest != catalog.digest
    ):
        raise ValueError("catalog entry identity changed")
