---
title: CAPSEM Proxy Setup
description: How to set up and run the CAPSEM proxy for transparent LLM API security.
---

The CAPSEM proxy provides transparent security monitoring and control for OpenAI and Google Gemini API requests. It acts as a drop-in replacement for your LLM API base URLs, enabling real-time security policy enforcement without modifying your application code.

## Features

- **Multi-Provider Support**: Proxies both OpenAI and Google Gemini APIs
- **Transparent Integration**: Works as a drop-in replacement - just change the base URL
- **Real-time Security**: CAPSEM policies enforced at multiple interception points
- **Streaming Support**: Full support for SSE streaming responses
- **Tool Calling**: Transparent proxy for tool/function calling
- **Multi-tenant**: API keys passed through from clients, never stored server-side

## Architecture

```
Your Application (OpenAI SDK / Gemini SDK)
    ↓
CAPSEM Proxy
    ↓ Security Checks (prompt, tools, response)
    ↓
OpenAI API / Gemini API
```

## Installation

### Setup a venv (recommended)
While optinal, we recommend using a virtual environment as this will ensure that the dependencies for CAPSEM are isolated from the rest of your system.

```bash
python -m venv .venv
source .venv/bin/activate  # On Windows use `.venv\Scripts\activate`
```

### Install CAPSEM Proxy
```bash
pip install capsem_proxy
```

## Running the Proxy

Start the proxy server:

```bash
# Using uvicorn directly
uvicorn capsem_proxy.server:app --host 127.0.0.1 --port 8000

# Or if you activated the venv
source .venv/bin/activate
uvicorn capsem_proxy.server:app --host 127.0.0.1 --port 8000
```

The proxy will start on `http://localhost:8000` and is ready to receive requests.

## Verify Installation

Check that the proxy is running:

```bash
curl http://localhost:8000/health
```

You should see:

```json
{
  "status": "healthy",
  "version": "0.1.0",
  "providers": ["openai", "gemini"]
}
```

## Security Policies

By default, the proxy uses CAPSEM's `DebugPolicy` which blocks:
- Prompts containing the `capsem_block` keyword
- Tools with `capsem_block` in their name

Blocked requests return HTTP 403 with details:

```json
{
  "detail": "Request blocked by security policy: Detected 'capsem_block' in prompt"
}
```

You can customize security policies by modifying `capsem_proxy/capsem_integration.py`.

## Next Steps

- [OpenAI Proxy Tutorial](/tutorials/openai-proxy/) - Learn how to proxy OpenAI API calls
- [Gemini Proxy Tutorial](/tutorials/gemini-proxy/) - Learn how to proxy Google Gemini API calls

## Troubleshooting

### Port Already in Use

If port 8000 is already in use, specify a different port:

```bash
uvicorn capsem_proxy.server:app --host 127.0.0.1 --port 8080
```

Remember to update your client's `base_url` accordingly.

### Connection Refused

Ensure the proxy is running and listening on the correct host/port. Check firewall settings if connecting from another machine.

### API Key Errors

The proxy passes through authentication to the actual LLM providers. Ensure your API keys are valid and have the necessary permissions.
