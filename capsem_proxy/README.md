# CAPSEM Proxy

Multi-tenant LLM proxy with CAPSEM security policy enforcement. Provides transparent security monitoring and control for OpenAI and Google Gemini API requests while supporting streaming responses and tool calling.

## Features

- **Multi-Provider Support**: OpenAI and Google Gemini API proxying
- **Multi-tenant Architecture**: API keys passed through from clients, never stored server-side
- **CAPSEM Security Integration**: Real-time security policy enforcement at multiple interception points
- **Streaming Support**: Full support for SSE streaming responses (both OpenAI and Gemini)
- **Tool Calling**: Transparent proxy for tool calling (client-side execution)
- **API Compatible**: Drop-in replacement for OpenAI and Gemini API base URLs
- **CORS Enabled**: Ready for web client integration

## Architecture

```
Client (OpenAI SDK / Gemini SDK / HTTP)
    ↓
CAPSEM Proxy (localhost:8000)
    ↓ CAPSEM Checks (prompt, tools, response)
    ↓
OpenAI API / Gemini API
```

### Security Interception Points

1. **on_model_call**: Validates prompts before sending to LLM provider
2. **on_tool_call**: Validates tool definitions
3. **on_model_response**: Validates responses from LLM provider

## Installation

```bash
# Install dependencies
uv sync

# Activate virtual environment
source .venv/bin/activate
```

## Configuration

Create a `.env` file in the `capsem/` directory with your API keys:

```env
OPENAI_API_KEY=sk-...
GEMINI_API_KEY=AIza...
```

You can use one or both providers depending on your needs.

## Usage

### Start the Proxy

```bash
uvicorn proxy.server:app --host 127.0.0.1 --port 8000
```

### Use with OpenAI SDK

```python
from openai import OpenAI

# Point to the proxy
client = OpenAI(
    api_key="your-openai-key",  # Your key, passed through
    base_url="http://localhost:8000/v1"
)

# Use normally
response = client.chat.completions.create(
    model="gpt-5-nano",
    messages=[{"role": "user", "content": "Hello!"}]
)
```

### Streaming Example

```python
stream = client.chat.completions.create(
    model="gpt-5-nano",
    messages=[{"role": "user", "content": "Count to 5"}],
    stream=True
)

for chunk in stream:
    if chunk.choices[0].delta.content:
        print(chunk.choices[0].delta.content, end="")
```

### Tool Calling Example

```python
tools = [{
    "type": "function",
    "function": {
        "name": "get_weather",
        "description": "Get weather for a location",
        "parameters": {
            "type": "object",
            "properties": {
                "location": {"type": "string"}
            }
        }
    }
}]

response = client.chat.completions.create(
    model="gpt-5-nano",
    messages=[{"role": "user", "content": "Weather in Paris?"}],
    tools=tools
)
```

### Use with Gemini SDK

```python
from google import genai

# Configure client to use the proxy
client = genai.Client(
    api_key="your-gemini-key",  # Your key, passed through
    http_options={'base_url': 'http://localhost:8000', 'timeout': 60000}
)

# Use normally
response = client.models.generate_content(
    model='gemini-2.0-flash-exp',
    contents='Hello!'
)
print(response.text)
```

### Use with Gemini (HTTP Client)

```python
import httpx

# Make requests directly to the proxy
response = httpx.post(
    "http://localhost:8000/v1beta/models/gemini-2.0-flash-exp:generateContent",
    headers={"x-goog-api-key": "your-gemini-key"},  # Your key, passed through
    json={
        "contents": [
            {
                "role": "user",
                "parts": [{"text": "Hello!"}]
            }
        ]
    }
)
```

### Gemini with Tools

```python
tools = [{
    "functionDeclarations": [{
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
    }]
}]

response = httpx.post(
    "http://localhost:8000/v1beta/models/gemini-2.0-flash-exp:generateContent",
    headers={"x-goog-api-key": "your-gemini-key"},
    json={
        "contents": [{
            "role": "user",
            "parts": [{"text": "Weather in Paris?"}]
        }],
        "tools": tools
    }
)
```

### Gemini Streaming

```python
async with httpx.AsyncClient() as client:
    async with client.stream(
        "POST",
        "http://localhost:8000/v1beta/models/gemini-2.0-flash-exp:streamGenerateContent",
        headers={"x-goog-api-key": "your-gemini-key"},
        json={
            "contents": [{
                "role": "user",
                "parts": [{"text": "Count to 5"}]
            }]
        }
    ) as response:
        async for chunk in response.aiter_bytes():
            print(chunk.decode("utf-8"))
```

## CAPSEM Security Policies

The proxy integrates with CAPSEM's DebugPolicy by default, which blocks:
- Prompts containing `capsem_block` keyword
- Tools with `capsem_block` in their name

Blocked requests return HTTP 403 with details:
```json
{
  "detail": "Request blocked by security policy: Detected 'capsem_block' in prompt"
}
```

## Testing

The test suite is organized into three categories for optimal development workflow:

### Test Categories

1. **Fast Tests** (run by default) - ~0.6s
   - Unit tests using FastAPI TestClient
   - Mock tests with fake LLM responses
   - Validation and error handling tests
   - No external dependencies required

2. **Integration Tests** (`@pytest.mark.integration`) - skipped by default
   - Tests requiring real API calls
   - Tests requiring proxy server running on localhost:8000
   - Requires valid API keys in `.env` file

### Running Tests

```bash
# Run fast tests only (DEFAULT - recommended for development)
# Good for quick feedback before git push
pytest -v

# Run ALL tests including integration tests
# Requires API keys and may require proxy server running
pytest -v -m ""

# Run only integration tests
pytest -v -m integration

# Run specific test file
pytest tests/test_openai_proxy_mock.py -v

# Run specific test
pytest tests/test_openai_proxy.py::test_health_check -v

# Run with detailed output
pytest -v -s
```

### Test Files

```
tests/
├── test_openai_proxy.py          # OpenAI integration tests
├── test_openai_proxy_mock.py     # OpenAI mock tests (fast)
├── test_gemini_proxy.py          # Gemini integration tests
└── test_gemini_proxy_mock.py     # Gemini mock tests (fast)
```

### Mock Tests

Mock tests use `unittest.mock` to fake httpx responses, allowing you to test:
- Request/response handling without real API calls
- CAPSEM security policy enforcement (blocks dangerous requests before reaching provider)
- Error handling and validation
- Tool/function calling flows

Example mock test verifying CAPSEM blocks dangerous prompts:
```python
def test_capsem_blocks_dangerous_prompt_mock(test_client, mock_httpx):
    """CAPSEM blocks prompts with 'capsem_block' keyword"""
    response = test_client.post(
        "/v1/chat/completions",
        headers={"Authorization": "Bearer sk-test-key"},
        json={
            "model": "gpt-5-nano",
            "messages": [{"role": "user", "content": "Tell me about capsem_block"}]
        }
    )

    # Verify blocked by CAPSEM (403), httpx never called
    assert response.status_code == 403
    assert "blocked by security policy" in response.json()["detail"].lower()
```

### Integration Tests

Integration tests verify end-to-end functionality with real APIs:
- Actual LLM responses
- Streaming responses
- Multi-turn tool calling
- CAPSEM blocking with real providers

**Requirements:**
- Valid API keys in `.env` file
- For some tests: proxy server running on `localhost:8000`

```bash
# Start proxy server (in separate terminal)
uvicorn capsem_proxy.server:app --host 127.0.0.1 --port 8000

# Run integration tests
pytest -v -m integration
```

### Test Configuration

Tests are configured in `pyproject.toml`:
```toml
[tool.pytest.ini_options]
markers = [
    "integration: requires proxy server or real API calls",
]
# By default, skip integration tests
addopts = "-m 'not integration'"
```

## API Endpoints

### Health Check
```
GET /health
```
Returns status and list of available providers

### OpenAI Endpoints

#### Chat Completions
```
POST /v1/chat/completions
```
OpenAI-compatible endpoint supporting:
- Non-streaming responses
- Streaming responses (SSE)
- Tool calling
- CAPSEM security checks

#### Responses API
```
POST /v1/responses
```
OpenAI Responses API endpoint (requires newer OpenAI SDK version)

### Gemini Endpoints

#### Generate Content
```
POST /v1beta/models/{model}:generateContent
```
Gemini API endpoint supporting:
- Non-streaming responses
- Function declarations (tools)
- CAPSEM security checks

#### Stream Generate Content
```
POST /v1beta/models/{model}:streamGenerateContent
```
Gemini streaming endpoint (SSE)

## Project Structure

```
capsem-proxy/
├── capsem_proxy/
│   ├── server.py              # FastAPI app
│   ├── api/
│   │   ├── openai.py          # OpenAI endpoints
│   │   └── gemini.py          # Gemini endpoints
│   ├── providers/
│   │   ├── openai.py          # OpenAI HTTP client
│   │   └── gemini.py          # Gemini HTTP client
│   ├── security/
│   │   └── identity.py        # API key hashing
│   └── capsem_integration.py  # CAPSEM SecurityManager
├── tests/
│   ├── test_openai_proxy.py       # OpenAI integration tests
│   ├── test_openai_proxy_mock.py  # OpenAI mock tests (fast)
│   ├── test_gemini_proxy.py       # Gemini integration tests
│   └── test_gemini_proxy_mock.py  # Gemini mock tests (fast)
└── pyproject.toml
```

## Multi-Tenant Design

- Each request is identified by a hashed `user_id` derived from the API key
- API keys are NEVER stored on the server
- All requests are logged with `user_id` for analytics
- CAPSEM policies apply per-user automatically

## Development

### Adding New Endpoints

1. Create endpoint in `capsem_proxy/api/openai.py` or `capsem_proxy/api/gemini.py`
2. Add CAPSEM security checks at appropriate interception points
3. Forward request to provider (using httpx)
4. Write tests:
   - Mock tests first (fast feedback)
   - Integration tests for end-to-end validation

### Adding New Providers

1. Create provider class in `capsem_proxy/providers/`
2. Implement HTTP client methods using `httpx.AsyncClient`
3. Add API router in `capsem_proxy/api/`
4. Register router in `capsem_proxy/server.py`
5. Write comprehensive test suite (mock + integration)

### Test-Driven Development Workflow

1. **Write mock tests first** - Fast feedback on logic without external dependencies
2. **Run tests frequently** - `pytest -v` runs in ~0.6s
3. **Add integration tests** - Verify end-to-end with real APIs
4. **Mark integration tests** - Use `@pytest.mark.integration` decorator
5. **CI/CD** - Fast tests run on every commit, integration tests on demand

## License

Copyright 2025 Google LLC

Licensed under the Apache License, Version 2.0


## Disclaimer

This is not an officially supported Google product. This project is not eligible for the [Google Open Source Software Vulnerability Rewards Program](https://bughunters.google.com/open-source-security).
