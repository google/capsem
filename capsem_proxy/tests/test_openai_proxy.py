# Copyright 2025 Google LLC
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Integration tests for OpenAI proxy

OpenAI Chat Completions API Documentation (from openai.md):

Request Parameters:
- messages: array (Required) - List of message objects with 'role' and 'content'
- model: string (Required) - Model ID like "gpt-4o" or "gpt-5-nano"
- tools: array (Optional) - List of tools the model may call
  - type: "function"
  - function: {name, description, parameters (JSON Schema)}
- tool_choice: string or object (Optional)
  - "none": model will not call any tool
  - "auto": model can pick between message or calling tools (DEFAULT when tools present)
  - "required": model must call one or more tools
  - {"type": "function", "function": {"name": "..."}} forces a specific tool
- max_completion_tokens: integer (Optional) - Max tokens for completion (replaces deprecated max_tokens)
- max_tokens: Deprecated - Use max_completion_tokens instead

Response Structure:
- choices[0].message:
  - role: "assistant"
  - content: text response (null when tool_calls present)
  - tool_calls: array of tool call requests
    - id: unique identifier (e.g., "call_abc123")
    - type: "function"
    - function: {name, arguments (JSON string)}
  - finish_reason: "stop" | "tool_calls" | "length"

Example Tool Response:
{
  "message": {
    "role": "assistant",
    "content": null,
    "tool_calls": [{
      "id": "call_abc123",
      "type": "function",
      "function": {
        "name": "get_current_weather",
        "arguments": "{\n\"location\": \"Boston, MA\"\n}"
      }
    }]
  },
  "finish_reason": "tool_calls"
}
"""

import pytest
import os
from pathlib import Path
from dotenv import load_dotenv
from openai import OpenAI
from fastapi.testclient import TestClient
from capsem_proxy.server import app

# Load environment from parent directory .env
env_path = Path(__file__).parent.parent.parent / ".env"
load_dotenv(env_path)

# Global model configuration
MODEL_NAME = "gpt-5-nano"
MAX_TOKENS = 4000  # Increased for gpt-5-nano to generate proper responses


@pytest.fixture
def openai_api_key():
    """Get OpenAI API key from capsem/.env"""
    key = os.getenv("OPENAI_API_KEY")
    if not key:
        pytest.skip("OPENAI_API_KEY not set in capsem/.env")
    return key


@pytest.fixture
def test_client():
    """FastAPI test client"""
    return TestClient(app)


def test_health_check(test_client):
    """Test health check endpoint"""
    response = test_client.get("/health")
    assert response.status_code == 200
    data = response.json()
    assert data["status"] == "healthy"
    assert "openai" in data["providers"]


def test_basic_chat_completion(test_client, openai_api_key):
    """Test basic chat completion through proxy"""
    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": f"Bearer {openai_api_key}"},
        json={
            "model": MODEL_NAME,
            "messages": [
                {"role": "user", "content": "Say 'proxy works' and nothing else"}
            ],
            "max_completion_tokens": MAX_TOKENS
        }
    )

    assert response.status_code == 200
    data = response.json()
    assert "choices" in data
    assert len(data["choices"]) > 0
    assert "message" in data["choices"][0]
    print(f"\nResponse: {data['choices'][0]['message']['content']}")


def test_missing_authorization(test_client):
    """Test request without Authorization header"""
    response = test_client.post(
        "/v1/chat/completions",
        json={
            "model": "gpt-3.5-turbo",
            "messages": [{"role": "user", "content": "test"}]
        }
    )

    assert response.status_code == 422  # FastAPI validation error


def test_invalid_authorization_format(test_client):
    """Test request with invalid Authorization format"""
    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "InvalidFormat"},
        json={
            "model": "gpt-3.5-turbo",
            "messages": [{"role": "user", "content": "test"}]
        }
    )

    assert response.status_code == 400
    assert "Invalid Authorization header" in response.json()["detail"]


def test_invalid_api_key(test_client):
    """Test request with invalid API key"""
    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "Bearer sk-invalid-key-12345"},
        json={
            "model": MODEL_NAME,
            "messages": [{"role": "user", "content": "test"}],
            "max_completion_tokens": MAX_TOKENS
        }
    )

    # Should get 401 from OpenAI
    assert response.status_code == 401


def test_chat_with_tool_definitions(test_client, openai_api_key):
    """Test chat completion with tool definitions"""
    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": f"Bearer {openai_api_key}"},
        json={
            "model": MODEL_NAME,
            "messages": [
                {"role": "user", "content": "What's the weather in Paris?"}
            ],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get weather for a location",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "location": {
                                    "type": "string",
                                    "description": "City name"
                                }
                            },
                            "required": ["location"]
                        }
                    }
                }
            ],
            "max_completion_tokens": MAX_TOKENS
        }
    )

    assert response.status_code == 200
    data = response.json()
    assert "choices" in data
    message = data["choices"][0]["message"]
    print(f"\nTool test - message: {message}")

    # Check if model called the tool
    if "tool_calls" in message:
        print(f"Model called tool: {message['tool_calls']}")


def test_user_id_hashing(openai_api_key):
    """Test that user_id is correctly generated from API key"""
    from capsem_proxy.security.identity import get_user_id_from_auth

    auth_header = f"Bearer {openai_api_key}"
    user_id = get_user_id_from_auth(auth_header)

    # Should be 16 character hash
    assert len(user_id) == 16
    assert user_id.isalnum()

    # Same key should give same hash
    user_id2 = get_user_id_from_auth(auth_header)
    assert user_id == user_id2

    # Different key should give different hash
    user_id3 = get_user_id_from_auth("Bearer sk-different-key")
    assert user_id != user_id3


@pytest.mark.integration
def test_openai_direct_access(openai_api_key):
    """Test direct OpenAI access to validate API key and model work"""
    client = OpenAI(api_key=openai_api_key)

    # Test that API key and model are valid
    response = client.chat.completions.create(
        model=MODEL_NAME,
        messages=[
            {"role": "user", "content": "Reply with just the word test"}
        ],
        max_completion_tokens=MAX_TOKENS
    )

    # Validate we got a response
    assert response.choices
    assert len(response.choices) > 0
    assert response.choices[0].message.content is not None

    content = response.choices[0].message.content
    assert "test" in content.lower()

    print(f"\nDirect OpenAI Response: '{content}'")
    print(f"Model used: {response.model}")
    print(f"Finish reason: {response.choices[0].finish_reason}")

@pytest.mark.integration
def test_openai_sdk_through_proxy(openai_api_key):
    """Test using OpenAI SDK client pointing to proxy running on localhost:8000"""
    client = OpenAI(
        api_key=openai_api_key,
        base_url="http://localhost:8000/v1"
    )

    response = client.chat.completions.create(
        model=MODEL_NAME,
        messages=[
            {"role": "user", "content": "Say 'SDK works' and nothing else"}
        ],
        max_completion_tokens=MAX_TOKENS
    )

    assert response.choices
    assert len(response.choices) > 0
    print(f"\nOpenAI SDK through proxy - Response: '{response.choices[0].message.content}'")
    print(f"Model: {response.model}")

    content = response.choices[0].message.content
    assert "sdk works" in content.lower()
    print(f"Finish reason: {response.choices[0].finish_reason}")
    print(f"Full response: {response}")


@pytest.mark.integration
def test_openai_sdk_with_tools_through_proxy(openai_api_key):
    """Test OpenAI SDK with tool calling through proxy"""
    client = OpenAI(
        api_key=openai_api_key,
        base_url="http://localhost:8000/v1"
    )

    response = client.chat.completions.create(
        model=MODEL_NAME,
        messages=[
            {"role": "user", "content": "What's the weather in Paris?"}
        ],
        tools=[
            {
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get current weather for a location",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "location": {
                                "type": "string",
                                "description": "City name"
                            }
                        },
                        "required": ["location"]
                    }
                }
            }
        ],
        max_completion_tokens=MAX_TOKENS
    )

    assert response.choices
    message = response.choices[0].message
    print(f"\nSDK tool test - content: '{message.content}'")
    print(f"Model: {response.model}")

    if message.tool_calls:
        print(f"Tool called: {message.tool_calls[0].function.name}")
        print(f"Arguments: {message.tool_calls[0].function.arguments}")
    assert message.tool_calls is not None
    assert len(message.tool_calls) > 0
    assert message.tool_calls[0].function.name == "get_weather"

# ============================================================================
# TDD Tests for CAPSEM Integration - These should FAIL until we implement
# ============================================================================

@pytest.mark.integration
def test_weather_tool_execution_through_proxy(openai_api_key):
    """
    TDD Test: Test multi-turn conversation with tool execution.

    The tool executes CLIENT-SIDE (in this test), proxy is just transparent.
    """
    import json

    # Define tool function (executes CLIENT-SIDE)
    def get_weather(location: str, unit: str = "celsius") -> dict:
        """Mock weather function - executes on client side"""
        return {
            "location": location,
            "temperature": 18 if unit == "celsius" else 64,
            "unit": unit,
            "condition": "sunny"
        }

    client = OpenAI(
        api_key=openai_api_key,
        base_url="http://localhost:8000/v1"
    )

    # Turn 1: Ask about weather
    messages = [{"role": "user", "content": "What's the weather in San Francisco?"}]

    response = client.chat.completions.create(
        model=MODEL_NAME,
        messages=messages,
        tools=[
            {
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get current weather for a location",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "location": {
                                "type": "string",
                                "description": "City name"
                            },
                            "unit": {
                                "type": "string",
                                "enum": ["celsius", "fahrenheit"],
                                "description": "Temperature unit"
                            }
                        },
                        "required": ["location"]
                    }
                }
            }
        ],
        max_completion_tokens=MAX_TOKENS
    )

    # Model should request tool call
    message = response.choices[0].message
    assert message.tool_calls is not None, "Model should request tool call"
    assert len(message.tool_calls) > 0
    tool_call = message.tool_calls[0]
    assert tool_call.function.name == "get_weather"

    print(f"\n[Turn 1] Model requested tool: {tool_call.function.name}")
    print(f"Tool arguments: {tool_call.function.arguments}")

    # CLIENT executes the tool
    tool_args = json.loads(tool_call.function.arguments)
    tool_result = get_weather(**tool_args)
    print(f"[CLIENT] Executed tool locally: {tool_result}")

    # Turn 2: Send tool result back to LLM
    messages.append(message.model_dump())
    messages.append({
        "role": "tool",
        "tool_call_id": tool_call.id,
        "content": json.dumps(tool_result)
    })

    response2 = client.chat.completions.create(
        model=MODEL_NAME,
        messages=messages,
        max_completion_tokens=MAX_TOKENS
    )

    # Model should provide natural language response
    final_message = response2.choices[0].message
    assert final_message.content is not None
    print(f"\n[Turn 2] Final response: {final_message.content}")

    # Verify response mentions weather info
    content_lower = final_message.content.lower()
    assert "san francisco" in content_lower or "weather" in content_lower or "18" in final_message.content


@pytest.mark.integration
def test_prompt_with_block_keyword_blocked_by_capsem(openai_api_key):
    """
    Test: Prompt with 'capsem_block' keyword should be BLOCKED by CAPSEM DebugPolicy.

    CAPSEM DebugPolicy blocks any prompt containing "capsem_block" keyword.
    This test verifies the security policy is working.
    """
    from openai import PermissionDeniedError

    client = OpenAI(
        api_key=openai_api_key,
        base_url="http://localhost:8000/v1"
    )

    # Prompt contains "capsem_block" - DebugPolicy will block this
    with pytest.raises(PermissionDeniedError) as exc_info:
        response = client.chat.completions.create(
            model=MODEL_NAME,
            messages=[
                {"role": "user", "content": "Tell me about capsem_block technology"}
            ],
            max_completion_tokens=MAX_TOKENS
        )

    # Verify it's a 403 and mentions the security policy
    assert exc_info.value.status_code == 403
    assert "blocked by security policy" in str(exc_info.value).lower()

    print(f"\n✅ CAPSEM successfully blocked prompt with 'capsem_block' keyword")
    print(f"Error: {exc_info.value}")


@pytest.mark.integration
def test_tool_with_block_in_name_blocked_by_capsem(openai_api_key):
    """
    Test: Tool with 'capsem_block' in name should be BLOCKED by CAPSEM DebugPolicy.

    CAPSEM DebugPolicy blocks any tool with "capsem_block" in the name.
    This test verifies the security policy is working for tools.
    """
    from openai import PermissionDeniedError

    client = OpenAI(
        api_key=openai_api_key,
        base_url="http://localhost:8000/v1"
    )

    # Define a tool with "capsem_block" in the name - DebugPolicy will block this
    with pytest.raises(PermissionDeniedError) as exc_info:
        response = client.chat.completions.create(
            model=MODEL_NAME,
            messages=[
                {"role": "user", "content": "Use the dangerous_capsem_block tool"}
            ],
            tools=[
                {
                    "type": "function",
                    "function": {
                        "name": "dangerous_capsem_block",
                        "description": "A dangerous operation that should be blocked",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "description": "The action to perform"
                                }
                            },
                            "required": ["action"]
                        }
                    }
                }
            ],
            max_completion_tokens=100
        )

    # Verify it's a 403 and mentions the security policy
    assert exc_info.value.status_code == 403
    assert "blocked by security policy" in str(exc_info.value).lower()

    print(f"\n✅ CAPSEM successfully blocked tool with 'capsem_block' in name")
    print(f"Error: {exc_info.value}")


# ============================================================================
# Responses API Tests - Testing the newer Responses API
# ============================================================================

@pytest.mark.integration
def test_responses_api_basic_through_proxy(openai_api_key):
    """Test basic Responses API request through proxy"""
    client = OpenAI(
        api_key=openai_api_key,
        base_url="http://localhost:8000/v1"
    )

    response = client.responses.create(
        model=MODEL_NAME,
        input=[
            {"role": "user", "content": "Say 'responses api works' and nothing else"}
        ],
    )

    assert response.status == "completed"
    assert response.model == "gpt-5-nano-2025-08-07"
    print(f"\nResponses API - Status: {response.status}")
    print(f"Model: {response.model}")
    print(f"Output: {response.output}")


@pytest.mark.integration
def test_responses_api_with_tools_through_proxy(openai_api_key):
    """Test Responses API with tool calling through proxy"""
    client = OpenAI(
        api_key=openai_api_key,
        base_url="http://localhost:8000/v1"
    )

    tools = [
        {
            "type": "function",
            "name": "get_horoscope",
            "description": "Get today's horoscope for an astrological sign.",
            "parameters": {
                "type": "object",
                "properties": {
                    "sign": {
                        "type": "string",
                        "description": "An astrological sign like Taurus or Aquarius",
                    },
                },
                "required": ["sign"],
            },
        },
    ]

    response = client.responses.create(
        model=MODEL_NAME,
        tools=tools,
        input=[
            {"role": "user", "content": "What is my horoscope for Aquarius?"}
        ],
    )

    assert response.status == "completed"
    print(f"\nResponses API with tools - Status: {response.status}")
    print(f"Model: {response.model}")
    print(f"Output: {response.output}")

    # Assert that tool was called
    tool_calls = [
        item for item in response.output
        if hasattr(item, "type") and item.type == "function_call"
    ]

    assert len(tool_calls) > 0, "Expected tool call but none found"
    assert tool_calls[0].name == "get_horoscope"
    assert tool_calls[0].status == "completed"

    print(f"Tool called: {tool_calls[0].name}")
    print(f"Arguments: {tool_calls[0].arguments}")
    print(f"Call ID: {tool_calls[0].call_id}")


@pytest.mark.integration
def test_streaming_chat_completion_through_proxy(openai_api_key):
    """Test streaming chat completion through proxy"""
    client = OpenAI(
        api_key=openai_api_key,
        base_url="http://localhost:8000/v1"
    )

    stream = client.chat.completions.create(
        model=MODEL_NAME,
        messages=[
            {"role": "user", "content": "Count from 1 to 5, one number per line"}
        ],
        max_completion_tokens=MAX_TOKENS,
        stream=True
    )

    # Collect all chunks
    chunks = []
    content_pieces = []

    for chunk in stream:
        chunks.append(chunk)
        # Extract delta content if present
        if chunk.choices and len(chunk.choices) > 0:
            delta = chunk.choices[0].delta
            if delta.content:
                content_pieces.append(delta.content)
                print(delta.content, end="", flush=True)

    print()  # Newline after streaming

    # Verify we got chunks
    assert len(chunks) > 0, "Should receive at least one chunk"
    print(f"\nReceived {len(chunks)} chunks")

    # Verify we accumulated content
    full_content = "".join(content_pieces)
    assert len(full_content) > 0, "Should have accumulated content"
    print(f"Full streamed content: {full_content}")

    # Verify chunks have the expected structure
    first_chunk = chunks[0]
    assert hasattr(first_chunk, "id")
    assert hasattr(first_chunk, "model")
    assert first_chunk.model == "gpt-5-nano-2025-08-07"
    print(f"Stream ID: {first_chunk.id}")
    print(f"Model: {first_chunk.model}")

    # Last chunk should have finish_reason
    last_chunk = chunks[-1]
    if last_chunk.choices and len(last_chunk.choices) > 0:
        finish_reason = last_chunk.choices[0].finish_reason
        if finish_reason:
            print(f"Finish reason: {finish_reason}")
            assert finish_reason in ["stop", "length", "tool_calls"]
