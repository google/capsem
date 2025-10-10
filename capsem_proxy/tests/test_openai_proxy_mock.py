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

"""Mock tests for OpenAI proxy - testing internal logic without real API calls

These tests mock the httpx client to test:
- Request/response handling
- CAPSEM security policy enforcement
- Error handling
- Tool call processing
"""

import pytest
from unittest.mock import AsyncMock, MagicMock, patch
from fastapi.testclient import TestClient
from capsem_proxy.server import app


@pytest.fixture
def test_client():
    """FastAPI test client"""
    return TestClient(app)


@pytest.fixture
def mock_httpx():
    """Mock httpx.AsyncClient for OpenAI provider"""
    with patch("capsem_proxy.providers.openai.httpx.AsyncClient") as mock:
        yield mock


def setup_mock_response(mock_httpx, response_data):
    """Helper to setup mock httpx response"""
    mock_response = MagicMock()
    mock_response.json = MagicMock(return_value=response_data)
    mock_response.raise_for_status = MagicMock()
    mock_response.status_code = 200

    mock_client = AsyncMock()
    mock_client.__aenter__.return_value = mock_client
    mock_client.__aexit__.return_value = None
    mock_client.post = AsyncMock(return_value=mock_response)
    mock_httpx.return_value = mock_client
    return mock_client


def test_basic_chat_completion_mocked(test_client, mock_httpx):
    """Test basic chat completion with mocked httpx"""
    setup_mock_response(mock_httpx, {
        "id": "chatcmpl-test123",
        "object": "chat.completion",
        "created": 1234567890,
        "model": "gpt-5-nano",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Mocked response"
            },
            "finish_reason": "stop"
        }]
    })

    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "Bearer sk-test-key"},
        json={
            "model": "gpt-5-nano",
            "messages": [{"role": "user", "content": "Hello"}]
        }
    )

    assert response.status_code == 200
    data = response.json()
    assert data["choices"][0]["message"]["content"] == "Mocked response"


def test_tool_calls_mocked(test_client, mock_httpx):
    """Test tool calling with mocked response"""
    setup_mock_response(mock_httpx, {
        "id": "chatcmpl-test456",
        "object": "chat.completion",
        "created": 1234567890,
        "model": "gpt-5-nano",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": None,
                "tool_calls": [{
                    "id": "call_abc123",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": '{"location": "Paris"}'
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })

    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "Bearer sk-test-key"},
        json={
            "model": "gpt-5-nano",
            "messages": [{"role": "user", "content": "Weather in Paris?"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "location": {"type": "string"}
                        }
                    }
                }
            }]
        }
    )

    assert response.status_code == 200
    data = response.json()
    assert data["choices"][0]["message"]["tool_calls"][0]["function"]["name"] == "get_weather"


def test_capsem_blocks_dangerous_prompt_mock(test_client, mock_httpx):
    """Test CAPSEM blocks dangerous prompts before reaching provider"""
    mock_client = setup_mock_response(mock_httpx, {})

    # Request with 'capsem_block' keyword - should be blocked
    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "Bearer sk-test-key"},
        json={
            "model": "gpt-5-nano",
            "messages": [{"role": "user", "content": "Tell me about capsem_block"}]
        }
    )

    # Verify blocked by CAPSEM
    assert response.status_code == 403
    data = response.json()
    assert "blocked by security policy" in data["detail"].lower()

    # Verify httpx was NOT called (blocked before reaching provider)
    mock_client.post.assert_not_called()


def test_capsem_blocks_dangerous_tool_mock(test_client, mock_httpx):
    """Test CAPSEM blocks dangerous tools before reaching provider"""
    mock_client = setup_mock_response(mock_httpx, {})

    # Request with dangerous tool name
    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "Bearer sk-test-key"},
        json={
            "model": "gpt-5-nano",
            "messages": [{"role": "user", "content": "Use the tool"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "dangerous_capsem_block",
                    "description": "Dangerous",
                    "parameters": {"type": "object", "properties": {}}
                }
            }]
        }
    )

    # Verify blocked
    assert response.status_code == 403
    data = response.json()
    assert "blocked by security policy" in data["detail"].lower()

    # Verify httpx was NOT called
    mock_client.post.assert_not_called()


def test_capsem_allows_safe_request_mock(test_client, mock_httpx):
    """Test CAPSEM allows safe requests through"""
    mock_client = setup_mock_response(mock_httpx, {
        "id": "chatcmpl-test789",
        "object": "chat.completion",
        "created": 1234567890,
        "model": "gpt-5-nano",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Safe response"
            },
            "finish_reason": "stop"
        }]
    })

    # Safe request (no dangerous keywords)
    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "Bearer sk-test-key"},
        json={
            "model": "gpt-5-nano",
            "messages": [{"role": "user", "content": "Tell me about Python"}]
        }
    )

    # Verify allowed through
    assert response.status_code == 200
    data = response.json()
    assert data["choices"][0]["message"]["content"] == "Safe response"

    # Verify httpx WAS called
    mock_client.post.assert_called_once()


def test_error_handling_mock(test_client, mock_httpx):
    """Test error handling when httpx raises exception"""
    mock_client = AsyncMock()
    mock_client.__aenter__.return_value = mock_client
    mock_client.__aexit__.return_value = None
    mock_client.post = AsyncMock(side_effect=Exception("Service error"))
    mock_httpx.return_value = mock_client

    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "Bearer sk-test-key"},
        json={
            "model": "gpt-5-nano",
            "messages": [{"role": "user", "content": "Hello"}]
        }
    )

    # Verify error is handled
    assert response.status_code == 500


def test_user_id_extraction():
    """Test user ID extraction and hashing"""
    from capsem_proxy.security.identity import get_user_id_from_auth

    user_id_1 = get_user_id_from_auth("Bearer sk-test-key-123")
    user_id_2 = get_user_id_from_auth("Bearer sk-test-key-123")
    user_id_3 = get_user_id_from_auth("Bearer sk-different-key")

    # Same key produces same hash
    assert user_id_1 == user_id_2
    assert len(user_id_1) == 16
    assert user_id_1.isalnum()

    # Different key produces different hash
    assert user_id_1 != user_id_3


def test_request_validation():
    """Test validation of malformed requests"""
    test_client = TestClient(app)

    # Missing Authorization header
    response = test_client.post(
        "/v1/chat/completions",
        json={
            "model": "gpt-5-nano",
            "messages": [{"role": "user", "content": "test"}]
        }
    )
    assert response.status_code == 422

    # Invalid Authorization format
    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "InvalidFormat"},
        json={
            "model": "gpt-5-nano",
            "messages": [{"role": "user", "content": "test"}]
        }
    )
    assert response.status_code == 400


def test_multiple_tools_capsem_checks(test_client, mock_httpx):
    """Test CAPSEM checks all tools in request"""
    mock_client = setup_mock_response(mock_httpx, {})

    # Multiple tools, one dangerous
    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "Bearer sk-test-key"},
        json={
            "model": "gpt-5-nano",
            "messages": [{"role": "user", "content": "Use tools"}],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "safe_tool",
                        "description": "Safe",
                        "parameters": {"type": "object", "properties": {}}
                    }
                },
                {
                    "type": "function",
                    "function": {
                        "name": "danger_capsem_block",
                        "description": "Dangerous",
                        "parameters": {"type": "object", "properties": {}}
                    }
                }
            ]
        }
    )

    # Should be blocked
    assert response.status_code == 403
    mock_client.post.assert_not_called()
