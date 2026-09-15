"""Observe the real Ironbank MCP exchange through the Python SDK."""

import asyncio
import os

from capsem import VM, models


async def main() -> None:
    async with VM(os.environ["SDK_GATEWAY_URL"], os.environ["SDK_GATEWAY_TOKEN"],
                  id=os.environ["SDK_VM_ID"]) as vm:
        report = (await vm.stats.details()).interactions
        item = next(item for item in report.items if item.event_id == os.environ["SDK_MCP_EVENT_ID"])
        assert item.trace_id and item.model_call_id is None and item.model_event_id is None
        call = item.content
        assert isinstance(call, models.InteractionToolCall)
        assert call.origin is models.ToolOrigin.MCP and call.tool_name == "fixture_lookup"
        assert call.arguments is not None and isinstance(call.arguments.content, models.JsonContent)
        assert call.arguments.content.value == {"query": os.environ["SDK_MCP_NONCE"]}
        assert call.request is not None and isinstance(call.request.content, models.JsonContent)
        assert isinstance(call.request.content.value, dict)
        assert call.request.content.value["method"] == "tools/call"
        result = call.result
        assert result is not None and result.is_error is False and result.error_message is None
        assert result.payload is not None and isinstance(result.payload.content, models.JsonContent)
        assert isinstance(result.payload.content.value, dict)
        assert result.payload.content.value["content"] == [
            {"type": "text", "text": "capsem-mock-server:mcp:fixture_lookup"},
        ]
        assert result.response is not None and isinstance(result.response.content, models.JsonContent)
        assert isinstance(result.response.content.value, dict)
        assert result.response.content.value["jsonrpc"] == "2.0"
        print("SDK_MCP_ACCEPTANCE_OK")


if __name__ == "__main__":
    asyncio.run(main())
