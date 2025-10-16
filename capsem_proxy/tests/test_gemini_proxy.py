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

"""Integration tests for Gemini proxy

Google Gemini API Documentation:

Request Format:
- Endpoint: POST /v1beta/models/{model}:generateContent
- Auth: x-goog-api-key header
- Request body:
  - contents: array of content objects
    - role: "user" | "model" | "system"
    - parts: array of part objects
      - text: string content
  - tools: array (optional) - Function declarations
    - functionDeclarations: array
      - name: string
      - description: string
      - parameters: JSON Schema object
  - generationConfig: object (optional)

Response Structure:
- candidates: array
  - content: object
    - role: "model"
    - parts: array
      - text: string
  - finishReason: string

Streaming:
- Endpoint: POST /v1beta/models/{model}:streamGenerateContent
- Returns SSE stream with same response format
"""

import pytest
import os
from pathlib import Path
from dotenv import load_dotenv
from fastapi.testclient import TestClient
from capsem_proxy.server import app
import httpx
from google.genai import Client, types
from google.genai.errors import ClientError

# Load environment from parent directory .env
env_path = Path(__file__).parent.parent.parent / ".env"
load_dotenv(env_path)

# Global model configuration
MODEL_NAME = "gemini-2.5-flash"
CAPSEM_PROXY = "http://127.0.0.1:8000"

@pytest.fixture
def gemini_api_key():
    """Get Gemini API key from capsem/.env"""
    key = os.getenv("GEMINI_API_KEY")
    if not key:
        pytest.skip("GEMINI_API_KEY not set in environment")
    return key


@pytest.fixture
def test_client():
    """FastAPI test client"""
    return TestClient(app)


@pytest.fixture()
def proxied_client():
    """Google Gemini SDK client"""
    key = os.getenv("GEMINI_API_KEY")
    if not key:
        pytest.skip("GEMINI_API_KEY not set in environment")
    http_options = types.HttpOptions(base_url=CAPSEM_PROXY)
    client = Client(http_options=http_options)
    return client

def test_health_check_includes_gemini(test_client):
    """Test health check endpoint includes gemini provider"""
    response = test_client.get("/health")
    assert response.status_code == 200
    data = response.json()
    assert data["status"] == "healthy"
    assert "gemini" in data["providers"]


@pytest.mark.integration
def test_basic_generate_content(test_client, gemini_api_key):
    """Test basic generateContent through proxy"""
    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": gemini_api_key},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Say 'proxy works' and nothing else"}],
                }
            ]
        },
    )

    assert response.status_code == 200
    data = response.json()
    assert "candidates" in data
    assert len(data["candidates"]) > 0
    assert "content" in data["candidates"][0]
    content = data["candidates"][0]["content"]
    assert "parts" in content
    assert len(content["parts"]) > 0
    text = content["parts"][0]["text"]
    print(f"\nResponse: {text}")


def test_missing_api_key_header(test_client):
    """Test request without x-goog-api-key header"""
    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "test"}],
                }
            ]
        },
    )

    assert response.status_code == 422  # FastAPI validation error


@pytest.mark.integration
def test_invalid_api_key(test_client):
    """Test request with invalid API key"""
    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": "invalid-key-12345"},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "test"}],
                }
            ]
        },
    )

    # Should get error from Gemini API
    assert response.status_code in [400, 401, 403]


@pytest.mark.integration
def test_generate_content_with_tools(test_client, gemini_api_key):
    """Test generateContent with function declarations"""
    response = test_client.post(
        f"/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": gemini_api_key},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "What's the weather in Paris?"}],
                }
            ],
            "tools": [
                {
                    "functionDeclarations": [
                        {
                            "name": "get_weather",
                            "description": "Get current weather for a location",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "location": {
                                        "type": "string",
                                        "description": "City name",
                                    }
                                },
                                "required": ["location"],
                            },
                        }
                    ]
                }
            ],
        },
    )

    assert response.status_code == 200
    data = response.json()
    print(f"\nTool test response: {data}")


@pytest.mark.integration
def test_gemini_direct_access(gemini_api_key):
    """Test direct Gemini API access to validate API key and model work"""
    import httpx

    response = httpx.post(
        f"https://generativelanguage.googleapis.com/v1beta/models/{MODEL_NAME}:generateContent",
        headers={"x-goog-api-key": gemini_api_key},
        json={
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Reply with just the word test"}],
                }
            ]
        },
        timeout=30.0,
    )

    assert response.status_code == 200
    data = response.json()
    assert "candidates" in data
    assert len(data["candidates"]) > 0

    text = data["candidates"][0]["content"]["parts"][0]["text"]
    print(f"\nDirect Gemini Response: '{text}'")


@pytest.mark.integration
async def test_gemini_sdk_through_proxy(gemini_api_key):
    """Test using google-genai SDK client pointing to proxy"""
    # Use httpx to test the proxy (google-genai SDK would need base_url support)
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"http://localhost:8000/v1beta/models/{MODEL_NAME}:generateContent",
            headers={"x-goog-api-key": gemini_api_key},
            json={
                "contents": [
                    {
                        "role": "user",
                        "parts": [{"text": "Say 'SDK works' and nothing else"}],
                    }
                ]
            },
            timeout=30.0,
        )

    assert response.status_code == 200
    data = response.json()
    text = data["candidates"][0]["content"]["parts"][0]["text"]
    print(f"\nGemini through proxy - Response: '{text}'")
    assert "sdk works" in text.lower() or "works" in text.lower()

@pytest.mark.integration
async def test_streaming_generate_content(gemini_api_key):
    """Test streaming generateContent through proxy"""
    async with httpx.AsyncClient() as client:
        async with client.stream(
            "POST",
            f"http://localhost:8000/v1beta/models/{MODEL_NAME}:streamGenerateContent",
            headers={"x-goog-api-key": gemini_api_key},
            json={
                "contents": [
                    {
                        "role": "user",
                        "parts": [{"text": "Say hello"}],
                    }
                ]
            },
            timeout=30.0,
        ) as response:
            assert response.status_code == 200

            # Collect streaming chunks
            chunks = []
            try:
                async for chunk in response.aiter_bytes():
                    chunks.append(chunk)
                    print(chunk.decode("utf-8"), end="", flush=True)
            except httpx.RemoteProtocolError:
                # Connection may close early after last chunk, this is okay
                pass

            print()  # Newline after streaming

            # Verify we got at least one chunk
            assert len(chunks) > 0, "Should receive at least one chunk"
            print(f"\nReceived {len(chunks)} chunks")


def test_user_id_hashing(gemini_api_key):
    """Test that user_id is correctly generated from Gemini API key"""
    from capsem_proxy.security.identity import get_user_id_from_auth

    # Gemini uses x-goog-api-key, but we wrap it as Bearer for hashing
    auth_header = f"Bearer {gemini_api_key}"
    user_id = get_user_id_from_auth(auth_header)

    # Should be 16 character hash
    assert len(user_id) == 16
    assert user_id.isalnum()

    # Same key should give same hash
    user_id2 = get_user_id_from_auth(auth_header)
    assert user_id == user_id2

    # Different key should give different hash
    user_id3 = get_user_id_from_auth("Bearer different-key")
    assert user_id != user_id3



@pytest.mark.integration
async def test_sdk_with_tool(proxied_client: Client):
    """Test google-genai SDK with tool declaration transparently work through
    proxy"""

    def weather_function(location: str) -> dict:
        return {"temperature": "20°C", "location": location}

    config = types.GenerateContentConfig(tools=[weather_function])

    response = proxied_client.models.generate_content(
        model=MODEL_NAME,
        contents="what is the weather in paris?",
        config=config,
    )
    print(response.text)
    assert response.text
    assert "20" in response.text


@pytest.mark.integration
async def test_sdk_block_tool_name(proxied_client: Client):
    """Test on_tool_call blocking by tool name works"""

    def weather_capsem_block_function(location: str) -> dict:
        return {"temperature": "20°C", "location": location}

    config = types.GenerateContentConfig(tools=[weather_capsem_block_function])

    # Assert that ClientError is raised
    with pytest.raises(ClientError) as exc_info:
        response = proxied_client.models.generate_content(
            model=MODEL_NAME,
            contents="what is the weather in paris?",
            config=config,
        )
    assert "capsem_block" in str(exc_info.value)
    assert "security policy" in str(exc_info.value)

@pytest.mark.integration
async def test_sdk_block_tool_return(proxied_client: Client):
    """Test on_tool_result blocks"""

    def weather_function(location: str) -> dict:
        return {"temperature": "20°C", "location": 'capsem_block'}

    config = types.GenerateContentConfig(tools=[weather_function])

    # Assert that ClientError is raised
    with pytest.raises(ClientError) as exc_info:
        response = proxied_client.models.generate_content(
            model=MODEL_NAME,
            contents="what is the weather in paris?",
            config=config,
        )
    assert "capsem_block" in str(exc_info.value)
    assert "security policy" in str(exc_info.value)


@pytest.mark.integration
async def test_sdk_block_prompt(proxied_client: Client):
    """Test on_tool_result blocks"""

    def weather_function(location: str) -> dict:
        return {"temperature": "20°C", "location": location}

    config = types.GenerateContentConfig(tools=[weather_function])

    # Assert that ClientError is raiseds
    with pytest.raises(ClientError) as exc_info:
        response = proxied_client.models.generate_content(
            model=MODEL_NAME,
            contents="what is the weather in paris? capsem_block",
            config=config,
        )
    assert "capsem_block" in str(exc_info.value)
    assert "security policy" in str(exc_info.value)