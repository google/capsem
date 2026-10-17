"""Official Python OpenAI client probe for the standalone Capsem endpoint."""

from __future__ import annotations

import argparse
import asyncio
import json

from openai import APIStatusError, AsyncOpenAI
from openai.types.chat import (
    ChatCompletionFunctionToolParam,
    ChatCompletionMessageFunctionToolCall,
)
from openai.types.responses import FunctionToolParam


async def probe(base_url: str) -> dict[str, object]:
    client = AsyncOpenAI(
        api_key="sk-capsem-standalone-python",
        base_url=base_url,
        max_retries=0,
        timeout=5.0,
    )
    tools: list[ChatCompletionFunctionToolParam] = [
        {
            "type": "function",
            "function": {
                "name": "fixture_lookup",
                "description": "Read the deterministic fixture.",
                "parameters": {
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"],
                },
            },
        }
    ]
    chat = await client.chat.completions.create(
        model="gpt-fixture",
        messages=[{"role": "user", "content": "Use the fixture."}],
        tools=tools,
    )
    tool_calls = chat.choices[0].message.tool_calls
    assert tool_calls is not None
    tool_call = tool_calls[0]
    assert isinstance(tool_call, ChatCompletionMessageFunctionToolCall)
    assert tool_call.function.name == "fixture_lookup"
    assert json.loads(tool_call.function.arguments)["query"] == "Capsem ironbank poem"
    assert chat.usage is not None and chat.usage.total_tokens == 456

    chat_stream = await client.chat.completions.create(
        model="gpt-fixture",
        messages=[{"role": "user", "content": "Stream the fixture."}],
        stream=True,
    )
    streamed_chat = ""
    async for chunk in chat_stream:
        streamed_chat += chunk.choices[0].delta.content or ""
    assert streamed_chat == "Capsem ironbank poem"

    response_tools: list[FunctionToolParam] = [
        {
            "type": "function",
            "name": "exec_command",
            "description": "Run the deterministic fixture command.",
            "strict": False,
            "parameters": {
                "type": "object",
                "properties": {"cmd": {"type": "string"}},
                "required": ["cmd"],
            },
        }
    ]
    first = await client.responses.create(
        model="gpt-fixture",
        input="Call the fixture tool.",
        tools=response_tools,
    )
    function_call = next(item for item in first.output if item.type == "function_call")
    assert function_call.name == "exec_command"
    assert first.usage is not None and first.usage.total_tokens == 48

    response_stream = await client.responses.create(
        model="gpt-fixture",
        input=[
            {
                "type": "function_call_output",
                "call_id": function_call.call_id,
                "output": "Process exited with code 0",
            }
        ],
        tools=response_tools,
        stream=True,
    )
    response_text = ""
    response_usage = None
    async for event in response_stream:
        if event.type == "response.output_text.delta":
            response_text += event.delta
        elif event.type == "response.completed":
            response_usage = event.response.usage
    assert response_text == (
        "Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw"
    )
    assert response_usage is not None and response_usage.total_tokens == 12

    pending = asyncio.create_task(
        client.responses.create(model="capsem-sdk-cancel", input="wait")
    )
    await asyncio.sleep(0.2)
    pending.cancel()
    try:
        await pending
    except asyncio.CancelledError:
        cancelled = True
    else:
        cancelled = False
    assert cancelled

    after_cancel = await client.chat.completions.create(
        model="gpt-fixture",
        messages=[{"role": "user", "content": "Still alive?"}],
    )
    assert after_cancel.choices[0].message.content == (
        "Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw"
    )

    try:
        await client.responses.create(
            model="capsem-sdk-upstream-error",
            input="fail",
        )
    except APIStatusError as error:
        upstream_status = error.status_code
        assert isinstance(error.body, dict)
        upstream_code = error.body["code"]
    else:
        raise AssertionError("upstream failure was returned as success")
    assert upstream_status == 503
    assert upstream_code == "fixture_unavailable"
    await client.close()

    return {
        "client": "python",
        "chat_stream": streamed_chat,
        "chat_tool": tool_call.function.name,
        "chat_total_tokens": chat.usage.total_tokens,
        "responses_stream": response_text,
        "responses_tool": function_call.name,
        "responses_total_tokens": response_usage.total_tokens,
        "cancelled": cancelled,
        "upstream_status": upstream_status,
        "upstream_code": upstream_code,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", required=True)
    args = parser.parse_args()
    print(json.dumps(asyncio.run(probe(args.base_url)), sort_keys=True))


if __name__ == "__main__":
    main()
