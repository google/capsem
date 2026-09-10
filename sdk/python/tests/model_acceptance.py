"""Verify real model and tool ledger observations through typed SDK responses."""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path

from capsem import VM, models


async def main() -> None:
    async with VM(os.environ["SDK_GATEWAY_URL"], os.environ["SDK_GATEWAY_TOKEN"],
                  id=os.environ["SDK_VM_ID"], timeout=120) as vm:
        script = Path(os.environ["SDK_MODEL_SCRIPT"]).read_bytes()
        assert (await vm.copy.to_vm("sdk-model-proof.py", script)).success
        execution = await vm.exec("python3 /root/sdk-model-proof.py", timeout_secs=90)
        assert execution.exit_code == 0, execution.stderr
        observation = json.loads(next(line.removeprefix("IRONBANK_CLIENT_RESULT=")
                                      for line in execution.stdout.splitlines()
                                      if line.startswith("IRONBANK_CLIENT_RESULT=")))
        assert observation["file_matches"] and observation["output_contains_nonce"]
        assert await vm.copy.from_vm(observation["filename"]) == (observation["nonce"] + "\n").encode()
        async with asyncio.timeout(15):
            while True:
                detail = await vm.stats.details()
                calls = [event for event in detail.model_events if event.model == observation["model"]]
                tools = [event for event in detail.tool_events if event.tool_name == observation["tool_call_name"]]
                if len(calls) >= 2 and tools:
                    break
                await asyncio.sleep(0.1)
        assert all(event.provider == "openai" and event.path == "/v1/responses" and event.status_code == 200 for event in calls)
        assert all(event.event_id and event.input_tokens and event.output_tokens for event in calls)
        assert any(event.arguments and observation["nonce"] in event.arguments for event in tools)
        assert all(isinstance(event.decision, models.ToolDecision) and isinstance(event.source, models.ToolOrigin) for event in tools)
        interactions = detail.interactions.items
        typed_tools = [item for item in interactions if isinstance(item.content, models.InteractionToolCall)
                       and item.content.tool_name == observation["tool_call_name"]]
        assert typed_tools
        for item in typed_tools:
            call = item.content
            assert isinstance(call, models.InteractionToolCall)
            assert call.arguments is not None and isinstance(call.arguments.content, models.JsonContent)
            assert isinstance(call.arguments.content.value, dict)
            assert observation["nonce"] in json.dumps(call.arguments.content.value)
            assert item.model_event_id in {event.event_id for event in calls}
            assert item.trace_id and call.call_id
        assert any(isinstance(item.content, models.InteractionMessage) for item in interactions)
        assert any(isinstance(item.content, models.InteractionToolResult) for item in interactions)
        assert all(isinstance(body.payload.status, models.CaptureStatus) for body in detail.interactions.bodies)
        usage = next(row for row in detail.model_stats if row.model == observation["model"])
        assert usage.call_count >= 2 and usage.input_tokens > 0 and usage.output_tokens > 0
        assert usage.estimated_cost_usd > 0
        summary = await vm.stats.summary()
        assert summary.total_input_tokens >= usage.input_tokens
        assert summary.total_output_tokens >= usage.output_tokens
        assert summary.total_estimated_cost >= usage.estimated_cost_usd
        info = await vm.info()
        assert info.ai is not None and info.network is not None and info.files is not None
        print(f"SDK_MODEL_ACCEPTANCE_OK model={usage.model} calls={usage.call_count} tools={len(tools)} cost={usage.estimated_cost_usd}")


if __name__ == "__main__":
    asyncio.run(main())
