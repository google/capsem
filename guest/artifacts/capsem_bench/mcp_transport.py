"""The guest benchmark's MCP client: JSON-RPC over the relay's stdio.

The standard library rather than FastMCP: the runtime carries no pip packages.
The relay speaks newline-delimited JSON-RPC, and the load collector needs only
`initialize` and concurrent `tools/call` requests matched by id.
"""

import asyncio
import itertools
import json

PROTOCOL_VERSION = "2025-06-18"
# One response can carry a large tool result; asyncio's 64 KiB default
# line limit would end the reader on it.
LINE_LIMIT = 16 * 1024 * 1024


class McpError(RuntimeError):
    """A JSON-RPC error answer, or a relay that stopped answering."""


class StdioClient:
    """One MCP session over a subprocess's stdin/stdout."""

    def __init__(self, command, args=(), env=None):
        self._argv = [command, *args]
        self._env = env
        self._ids = itertools.count(1)
        self._pending = {}

    async def __aenter__(self):
        process = await asyncio.create_subprocess_exec(
            *self._argv,
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
            env=self._env,
            limit=LINE_LIMIT,
        )
        assert process.stdin is not None and process.stdout is not None
        self._process = process
        self._stdin = process.stdin
        self._stdout = process.stdout
        self._reader = asyncio.create_task(self._read())
        await self._request(
            "initialize",
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "capsem-bench", "version": "1.0"},
            },
        )
        await self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        return self

    async def __aexit__(self, *exc_info):
        self._stdin.close()
        try:
            await asyncio.wait_for(self._process.wait(), timeout=5)
        except TimeoutError:
            self._process.kill()
            await self._process.wait()
        self._reader.cancel()
        await asyncio.gather(self._reader, return_exceptions=True)

    async def call_tool(self, name, arguments):
        """Call one tool; raise `McpError` on an error answer or tool error."""
        result = await self._request("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise McpError(f"tool {name} returned an error: {result}")
        return result

    async def _send(self, message):
        self._stdin.write(json.dumps(message).encode() + b"\n")
        await self._stdin.drain()

    async def _request(self, method, params):
        identity = next(self._ids)
        answer = asyncio.get_running_loop().create_future()
        self._pending[identity] = answer
        await self._send({"jsonrpc": "2.0", "id": identity, "method": method, "params": params})
        message = await answer
        if "error" in message:
            raise McpError(f"{method} failed: {message['error']}")
        return message.get("result", {})

    async def _read(self):
        try:
            while line := await self._stdout.readline():
                message = json.loads(line)
                answer = self._pending.pop(message.get("id"), None)
                if answer is not None and not answer.done():
                    answer.set_result(message)
        finally:
            for answer in self._pending.values():
                if not answer.done():
                    answer.set_exception(McpError("the MCP relay closed its output"))
            self._pending.clear()
