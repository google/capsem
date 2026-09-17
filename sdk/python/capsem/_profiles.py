"""Typed profile and MCP resources over the authenticated gateway transport."""

from __future__ import annotations

import builtins

from . import _operations as api
from . import models
from ._transport import Transport


class ProfileMcp:
    def __init__(self, transport: Transport, profile_id: str) -> None:
        self._transport = transport
        self._profile_id = profile_id

    async def info(self) -> models.ProfileMcpInfoResponse:
        return await api.get_profile_mcp_info(self._transport, profile_id=self._profile_id)

    async def servers(self) -> models.McpServersListResponse:
        return await api.list_profile_mcp_servers(self._transport, profile_id=self._profile_id)

    async def default_permission(self) -> models.McpDefaultPermissionResponse:
        return await api.get_profile_mcp_default(self._transport, profile_id=self._profile_id)

    async def tools(self, server_id: str) -> models.McpToolsListResponse:
        return await api.list_profile_mcp_tools(
            self._transport, profile_id=self._profile_id, server_id=server_id,
        )

    async def refresh(self, server_id: str) -> models.McpRefreshResponse:
        return await api.refresh_profile_mcp_server(
            self._transport, profile_id=self._profile_id, server_id=server_id,
        )

    async def call(
        self, server_id: str, tool_id: str, arguments: models.Value,
    ) -> models.Value:
        return await api.call_profile_mcp_tool(
            self._transport,
            profile_id=self._profile_id,
            server_id=server_id,
            tool_id=tool_id,
            body=arguments,
        )


class Profiles:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def list(self) -> builtins.list[models.ProfileSummary]:
        return (await api.list_profiles(self._transport)).profiles

    def mcp(self, profile_id: str) -> ProfileMcp:
        return ProfileMcp(self._transport, profile_id)
