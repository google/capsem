---
title: OpenAI Proxying Demo
description: This is a demo of using CAPSEM to proxy requests to OpenAI via the CAPSEM
  Proxy. It shows how to set up a simple agent that uses OpenAI as its LLM, and how
  to enforce policies on the agent's behavior.
---
This is a demo of using CAPSEM to proxy requests to OpenAI via the CAPSEM Proxy. It shows how to set up a simple agent that uses OpenAI as its LLM, and how to enforce policies on the agent's behavior.


```python
import json
from openai import OpenAI
```

## Instantiate a Proxied OpenAI Client

To proxy the requests to OpenAI through CAPSEM, we need to pass the CAPSEM Proxy URL as the `base_url` when creating the OpenAI client.

We instantiate the OpenAI client with the CAPSEM Proxy URL, and the client will automatically use the API key from the environment.


```python
CAPSEM_PROXY = "http://127.0.0.1:8000"
MODEL_NAME = "gpt-5-nano"
client = OpenAI(base_url=f"{CAPSEM_PROXY}/v1")
```

## Calling OpenAI via CAPSEM Proxy

We can now use the OpenAI client as usual, and all requests will be proxied through CAPSEM. This allows us to enforce policies on the requests and responses, such as filtering out sensitive information or input sanitization.

Here is a simple example of generating content with OpenAI via CAPSEM Proxy with tool calling.


```python
# Define a tool
tools = [
    {
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get the current weather for a location",
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
]

# First turn: Model requests tool call
response1 = client.chat.completions.create(
    model=MODEL_NAME,
    messages=[
        {"role": "user", "content": "What's the weather in Paris?"}
    ],
    tools=tools
)

print("Model requested tool call:")
tool_call = response1.choices[0].message.tool_calls[0]
print(f"Tool: {tool_call.function.name}")
print(f"Arguments: {tool_call.function.arguments}")

# Execute tool (client-side)
def get_weather(location: str) -> dict:
    # Dummy implementation for illustration
    return {"temperature": "20°C", "location": location, "condition": "sunny"}

args = json.loads(tool_call.function.arguments)
tool_result = get_weather(**args)
print(f"\nTool result: {tool_result}")

# Second turn: Send tool result back
response2 = client.chat.completions.create(
    model=MODEL_NAME,
    messages=[
        {"role": "user", "content": "What's the weather in Paris?"},
        response1.choices[0].message.model_dump(),
        {
            "role": "tool",
            "tool_call_id": tool_call.id,
            "content": json.dumps(tool_result)
        }
    ]
)

print(f"\nFinal response: {response2.choices[0].message.content}")
```

    Model requested tool call:
    Tool: get_weather
    Arguments: {"location":"Paris"}
    
    Tool result: {'temperature': '20°C', 'location': 'Paris', 'condition': 'sunny'}
    
    Final response: Currently in Paris: sunny, about 20°C. Would you like an hourly forecast or a 7-day outlook?


## Example of Detections

### PII Detection in Tool Call

In this example, the agent calls a tool where the model's requested arguments contain PII information. CAPSEM proxy detects the PII in the tool call and blocks the request.


```python
# Define a contact lookup tool
tools = [
    {
        "type": "function",
        "function": {
            "name": "contact_lookup",
            "description": "Look up contact information by email",
            "parameters": {
                "type": "object",
                "properties": {
                    "email": {
                        "type": "string",
                        "description": "Email address to lookup"
                    }
                },
                "required": ["email"]
            }
        }
    }
]

try:
    response = client.chat.completions.create(
        model=MODEL_NAME,
        messages=[
            {"role": "user", "content": "test contact lookup tool for a test email"}
        ],
        tools=tools
    )
    print(response.choices[0].message.content)
except Exception as e:
    print(f"CAPSEM BLOCKED: {e}")
```

    CAPSEM BLOCKED: Error code: 403 - {'detail': "Request blocked by security policy: PII detected in tool 'contact_lookup' arguments: EMAIL_ADDRESS(count=1, score=1.00, action=BLOCK)"}


### PII Detection in Tool Result

In this example, the client sends a tool result that contains PII information. CAPSEM proxy detects the PII in the tool result and blocks the request.


```python
# First turn: Model requests weather
tools = [
    {
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get weather for a location",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                },
                "required": ["location"]
            }
        }
    }
]

response1 = client.chat.completions.create(
    model=MODEL_NAME,
    messages=[
        {"role": "user", "content": "What's the weather in Paris?"}
    ],
    tools=tools
)

tool_call = response1.choices[0].message.tool_calls[0]

# Second turn: Send tool result with PII
try:
    response2 = client.chat.completions.create(
        model=MODEL_NAME,
        messages=[
            {"role": "user", "content": "What's the weather in Paris?"},
            response1.choices[0].message.model_dump(),
            {
                "role": "tool",
                "tool_call_id": tool_call.id,
                "content": json.dumps({
                    "temperature": "20°C",
                    "location": "PII test@gmail.com"  # PII in tool result
                })
            }
        ]
    )
    print(response2.choices[0].message.content)
except Exception as e:
    print(f"CAPSEM BLOCKED: {e}")
```

    CAPSEM BLOCKED: Error code: 403 - {'detail': "Request blocked by security policy: PII detected in tool 'unknown' response: EMAIL_ADDRESS(count=1, score=1.00, action=BLOCK)"}


### PII Detection in Model Response

In this example, the model generates a response that contains PII information. CAPSEM proxy detects the PII in the response and blocks it.


```python
try:
    response = client.chat.completions.create(
        model=MODEL_NAME,
        messages=[
            {"role": "user", "content": "My name is Elie - generate me an idea for an email address @gmail.com"}
        ]
    )
    print(response.choices[0].message.content)
except Exception as e:
    print(f"CAPSEM BLOCKED: {e}")
```

    CAPSEM BLOCKED: Error code: 403 - {'detail': 'Request blocked by security policy: PII detected in model response: EMAIL_ADDRESS(count=13, score=1.00, action=BLOCK), PERSON(count=2, score=0.85, action=LOG)'}


### Debug Policy - Blocking Test Keyword in prompt
The CAPSEM proxy includes a debug policy that blocks any request containing the keyword `capsem_block`. We use it to show how to block requests that contains specific keywords.


```python
# Test blocking in prompt
try:
    response = client.chat.completions.create(
        model=MODEL_NAME,
        messages=[
            {"role": "user", "content": "Tell me about capsem_block technology"}
        ]
    )
    print(response.choices[0].message.content)
except Exception as e:
    print(f"CAPSEM BLOCKED PROMPT: {e}")
```

    CAPSEM BLOCKED PROMPT: Error code: 403 - {'detail': "Request blocked by security policy: Detected 'capsem_block' in prompt"}


## Tool name 
This example demonstrate how CAPSEM scan for tool name as well by adding the
debug keyword in the tool name and see it being blocked.


```python
# Test blocking in tool name
try:
    response = client.chat.completions.create(
        model=MODEL_NAME,
        messages=[
            {"role": "user", "content": "Use the dangerous tool"}
        ],
        tools=[
            {
                "type": "function",
                "function": {
                    "name": "dangerous_capsem_block",
                    "description": "A blocked tool",
                    "parameters": {
                        "type": "object",
                        "properties": {}
                    }
                }
            }
        ]
    )
    print(response.choices[0].message.content)
except Exception as e:
    print(f"CAPSEM BLOCKED TOOL NAME: {e}")
```

    CAPSEM BLOCKED TOOL NAME: Error code: 403 - {'detail': "Tool blocked by security policy: Detected 'capsem_block' in tool name"}

