"""The MCP servers every VM runs, over the authenticated gateway transport.

They come from settings.toml `[mcp]` with corp's laid over it, so every VM
sees the same servers.
"""

from __future__ import annotations

from . import _operations as api
from . import models
from ._transport import Transport


class McpTools:
    def __init__(self, transport: Transport, server_id: str) -> None:
        self._transport = transport
        self._server_id = server_id

    async def list(self) -> models.McpToolsListResponse:
        return await api.list_mcp_tools(self._transport, server_id=self._server_id)

    async def call(self, name: str, arguments: models.Value) -> models.Value:
        return await api.call_mcp_tool(
            self._transport, server_id=self._server_id, tool_id=name, body=arguments,
        )


class McpServer:
    def __init__(self, transport: Transport, info: models.McpServerInfoResponse) -> None:
        self.info = info
        self.tools = McpTools(transport, info.name)
        self._transport = transport

    async def refresh(self) -> models.McpRefreshResponse:
        return await api.refresh_mcp_server(self._transport, server_id=self.info.name)


class Mcp:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def info(self) -> models.McpInfoResponse:
        return await api.get_mcp_info(self._transport)

    async def servers(self) -> models.McpServersListResponse:
        return await api.list_mcp_servers(self._transport)

    async def default_permission(self) -> models.McpDefaultPermissionResponse:
        return await api.get_mcp_default(self._transport)

    async def get(self, name: str) -> McpServer:
        matches = [server for server in await self.servers() if server.name == name]
        if len(matches) != 1:
            raise LookupError(f"expected one MCP server named {name!r}, found {len(matches)}")
        return McpServer(self._transport, matches[0])
