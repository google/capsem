"""Typed profile and MCP resources over the authenticated gateway transport."""

from __future__ import annotations

import builtins

from . import _operations as api
from . import models
from ._transport import Transport


class McpTools:
    def __init__(self, transport: Transport, profile_id: str, server_id: str) -> None:
        self._transport = transport
        self._profile_id = profile_id
        self._server_id = server_id

    async def list(self) -> models.McpToolsListResponse:
        return await api.list_profile_mcp_tools(
            self._transport, profile_id=self._profile_id, server_id=self._server_id,
        )

    async def call(self, name: str, arguments: models.Value) -> models.Value:
        return await api.call_profile_mcp_tool(
            self._transport, profile_id=self._profile_id, server_id=self._server_id,
            tool_id=name, body=arguments,
        )


class McpServer:
    def __init__(self, transport: Transport, profile_id: str,
                 info: models.McpServerInfoResponse) -> None:
        self.info = info
        self.tools = McpTools(transport, profile_id, info.name)
        self._transport = transport
        self._profile_id = profile_id

    async def refresh(self) -> models.McpRefreshResponse:
        return await api.refresh_profile_mcp_server(
            self._transport, profile_id=self._profile_id, server_id=self.info.name,
        )


class ProfileMcp:
    def __init__(self, transport: Transport, profile: models.ProfileSummary) -> None:
        self._transport = transport
        self._profile_id = profile.id

    async def info(self) -> models.ProfileMcpInfoResponse:
        return await api.get_profile_mcp_info(self._transport, profile_id=self._profile_id)

    async def servers(self) -> models.McpServersListResponse:
        return await api.list_profile_mcp_servers(self._transport, profile_id=self._profile_id)

    async def default_permission(self) -> models.McpDefaultPermissionResponse:
        return await api.get_profile_mcp_default(self._transport, profile_id=self._profile_id)

    async def get(self, name: str) -> McpServer:
        matches = [server for server in await self.servers() if server.name == name]
        if len(matches) != 1:
            raise LookupError(f"expected one MCP server named {name!r}, found {len(matches)}")
        return McpServer(self._transport, self._profile_id, matches[0])


class Profiles:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def list(self) -> builtins.list[models.ProfileSummary]:
        return (await api.list_profiles(self._transport)).profiles

    def mcp(self, profile: models.ProfileSummary) -> ProfileMcp:
        if not isinstance(profile, models.ProfileSummary):
            raise TypeError("profile must be an object returned by capsem.profiles")
        return ProfileMcp(self._transport, profile)
