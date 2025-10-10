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

"""Mock tests for Gemini proxy - testing internal logic without real API calls

These tests mock the httpx client to test:
- Request/response handling
- CAPSEM security policy enforcement
- Error handling
- Function calling processing
"""

import pytest
from unittest.mock import AsyncMock, MagicMock, patch
from fastapi.testclient import TestClient
from capsem_proxy.server import app


MODEL_NAME = "gemini-2.0-flash-exp"


@pytest.fixture
def test_client():
    """FastAPI test client"""
    return TestClient(app)


@pytest.fixture
def mock_httpx_gemini():
    """Mock httpx.AsyncClient for Gemini provider"""
    with patch("capsem_proxy.providers.gemini.httpx.AsyncClient") as mock:
        yield mock


def setup_mock_gemini_response(mock_httpx, response_data):
    """Helper to setup mock httpx response for Gemini"""
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


def test_basic_generate_content_mocked(test_client, mock_httpx_gemini):
    """Test basic generateContent with mocked httpx"""
    setup_mock_gemini_response(mock_httpx_gemini, {
        "candidates": [
            {
                "content": {
                    "role": "model",
                    "parts": [{"text": "Mocked Gemini response"}]
                },
                "finishReason": "STOP"
            }
        ]
    })

    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": "test-api-key"},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Hello"}]
                }
            ]
        }
    )

    assert response.status_code == 200
    data = response.json()
    assert data["candidates"][0]["content"]["parts"][0]["text"] == "Mocked Gemini response"


def test_function_declarations_mocked(test_client, mock_httpx_gemini):
    """Test function declarations with mocked provider"""
    setup_mock_gemini_response(mock_httpx_gemini, {
        "candidates": [
            {
                "content": {
                    "role": "model",
                    "parts": [{"text": "Let me check the weather"}]
                },
                "finishReason": "STOP"
            }
        ]
    })

    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": "test-api-key"},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Weather in Paris?"}]
                }
            ],
            "tools": [
                {
                    "functionDeclarations": [
                        {
                            "name": "get_weather",
                            "description": "Get weather",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "location": {"type": "string"}
                                }
                            }
                        }
                    ]
                }
            ]
        }
    )

    assert response.status_code == 200


def test_capsem_blocks_dangerous_prompt_gemini_mock(test_client, mock_httpx_gemini):
    """Test CAPSEM blocks dangerous prompts in Gemini requests"""
    mock_client = setup_mock_gemini_response(mock_httpx_gemini, {})

    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": "test-api-key"},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Tell me about capsem_block"}]
                }
            ]
        }
    )

    # Verify blocked
    assert response.status_code == 403
    data = response.json()
    assert "blocked by security policy" in data["detail"].lower()

    # Verify httpx was NOT called
    mock_client.post.assert_not_called()


def test_capsem_blocks_dangerous_function_gemini_mock(test_client, mock_httpx_gemini):
    """Test CAPSEM blocks dangerous function declarations"""
    mock_client = setup_mock_gemini_response(mock_httpx_gemini, {})

    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": "test-api-key"},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Use tool"}]
                }
            ],
            "tools": [
                {
                    "functionDeclarations": [
                        {
                            "name": "dangerous_capsem_block",
                            "description": "Dangerous",
                            "parameters": {"type": "object", "properties": {}}
                        }
                    ]
                }
            ]
        }
    )

    # Verify blocked
    assert response.status_code == 403
    data = response.json()
    assert "blocked by security policy" in data["detail"].lower()

    # Verify httpx was NOT called
    mock_client.post.assert_not_called()


def test_capsem_allows_safe_gemini_request_mock(test_client, mock_httpx_gemini):
    """Test CAPSEM allows safe Gemini requests"""
    mock_client = setup_mock_gemini_response(mock_httpx_gemini, {
        "candidates": [
            {
                "content": {
                    "role": "model",
                    "parts": [{"text": "Safe Gemini response"}]
                },
                "finishReason": "STOP"
            }
        ]
    })

    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": "test-api-key"},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Tell me about Python"}]
                }
            ]
        }
    )

    # Verify allowed
    assert response.status_code == 200
    data = response.json()
    assert data["candidates"][0]["content"]["parts"][0]["text"] == "Safe Gemini response"

    # Verify httpx WAS called
    mock_client.post.assert_called_once()


def test_gemini_error_handling_mock(test_client, mock_httpx_gemini):
    """Test error handling when Gemini provider raises exception"""
    mock_client = AsyncMock()
    mock_client.__aenter__.return_value = mock_client
    mock_client.__aexit__.return_value = None
    mock_client.post = AsyncMock(side_effect=Exception("Service error"))
    mock_httpx_gemini.return_value = mock_client

    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": "test-api-key"},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Hello"}]
                }
            ]
        }
    )

    # Verify error is handled
    assert response.status_code == 500


def test_gemini_request_validation():
    """Test validation of malformed Gemini requests"""
    test_client = TestClient(app)

    # Missing x-goog-api-key header
    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "test"}]
                }
            ]
        }
    )
    assert response.status_code == 422


def test_multiple_functions_capsem_checks(test_client, mock_httpx_gemini):
    """Test CAPSEM checks all function declarations"""
    mock_client = setup_mock_gemini_response(mock_httpx_gemini, {})

    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": "test-api-key"},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Use functions"}]
                }
            ],
            "tools": [
                {
                    "functionDeclarations": [
                        {
                            "name": "safe_function",
                            "description": "Safe",
                            "parameters": {"type": "object", "properties": {}}
                        },
                        {
                            "name": "danger_capsem_block",
                            "description": "Dangerous",
                            "parameters": {"type": "object", "properties": {}}
                        }
                    ]
                }
            ]
        }
    )

    # Should be blocked
    assert response.status_code == 403
    mock_client.post.assert_not_called()


def test_gemini_user_id_extraction():
    """Test user ID extraction from Gemini API key"""
    from capsem_proxy.security.identity import get_user_id_from_auth

    user_id_1 = get_user_id_from_auth("Bearer test-gemini-key-123")
    user_id_2 = get_user_id_from_auth("Bearer test-gemini-key-123")
    user_id_3 = get_user_id_from_auth("Bearer different-gemini-key")

    # Same key produces same hash
    assert user_id_1 == user_id_2
    assert len(user_id_1) == 16
    assert user_id_1.isalnum()

    # Different key produces different hash
    assert user_id_1 != user_id_3
