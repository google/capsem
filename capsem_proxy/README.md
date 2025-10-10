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

```bash
# Run all tests (both OpenAI and Gemini)
pytest tests/ -v

# Run OpenAI tests only
pytest tests/test_openai_proxy.py -v

# Run Gemini tests only
pytest tests/test_gemini_proxy.py -v

# Run specific test
pytest tests/test_openai_proxy.py::test_streaming_chat_completion_through_proxy -v

# Run with output
pytest tests/ -v -s
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
│   │   ├── openai.py          # OpenAI client
│   │   └── gemini.py          # Gemini client
│   ├── security/
│   │   └── identity.py        # API key hashing
│   └── capsem_integration.py  # CAPSEM SecurityManager
├── tests/
│   ├── test_openai_proxy.py   # OpenAI tests
│   └── test_gemini_proxy.py   # Gemini tests
└── pyproject.toml
```

## Multi-Tenant Design

- Each request is identified by a hashed `user_id` derived from the API key
- API keys are NEVER stored on the server
- All requests are logged with `user_id` for analytics
- CAPSEM policies apply per-user automatically

## Development

### Adding New Endpoints

1. Create endpoint in `proxy/api/openai.py`
2. Add CAPSEM security checks
3. Forward to OpenAI provider
4. Write integration tests

### Adding New Providers

1. Create provider class in `proxy/providers/`
2. Implement `chat_completion()` and `chat_completion_stream()` methods
3. Add router in `proxy/server.py`

## License

Copyright 2025 Google LLC

Licensed under the Apache License, Version 2.0
