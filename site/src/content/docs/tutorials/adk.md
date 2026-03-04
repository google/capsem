---
title: ADK Tutorial
description: This tutorial demonstrates how to integrate CAPSEM with the [ADK agent
  framework](https://google.github.io/adk-docs/).
---
This tutorial demonstrates how to integrate CAPSEM with the [ADK agent framework](https://google.github.io/adk-docs/).

## Installation

Make sure you have followed the instructions in the [installation guide](/getting_started/installation/) to set up your environment.

You need to install the adk part of CAPSEM:

```bash
pip install capsem[adk]
```

## Limitations
Known Limitation: some of the callbacks on ADK are not working as expected due to inability to modify the context via InvokationContext objects e.g on_user_message. This is a limitation of ADK and should be fixed in future versions.

## Dependencies

### Standard Libraries


```python
%load_ext autoreload
%autoreload 2
import os
from uuid import uuid4
from dotenv import load_dotenv
load_dotenv()
```




    True



## CAPSEM imports


```python
# @title CAPSEM imports
from capsem import SecurityManager
from capsem.policies import DebugPolicy

# Run CAPSEM policy manager as an ADK plugin
from capsem.integrations.adk import CAPSEMPlugin

# Extend ADK Runner to block unsafe flows based on CAPSEM policies decisions
from capsem.integrations.adk import CAPSEMRunner
```


```python
# @title ADK imports
import os
import asyncio
from google.adk.agents import Agent, LlmAgent
from google.adk.models.lite_llm import LiteLlm  # cross-platform LLM interface

from google.adk.sessions import InMemorySessionService
from google.adk.runners import Runner
from google.genai import types # For creating message Content/Parts
```

## ADK setup

### Agents tools


```python
# @title Define the get_weather Tool
def get_weather(city: str) -> dict:
    """Retrieves the current weather report for a specified city.

    Args:
        city (str): The name of the city (e.g., "New York", "London", "Tokyo").

    Returns:
        dict: A dictionary containing the weather information.
              Includes a 'status' key ('success' or 'error').
              If 'success', includes a 'report' key with weather details.
              If 'error', includes an 'error_message' key.
    """
    print(f"--- Tool: get_weather called for city: {city} ---") # Log tool execution
    city_normalized = city.lower().replace(" ", "") # Basic normalization

    # Mock weather data
    mock_weather_db = {
        "newyork": {"status": "success", "report": "The weather in New York is sunny with a temperature of 25°C."},
        "london": {"status": "success", "report": "It's cloudy in London with a temperature of 15°C."},
        "tokyo": {"status": "success", "report": "Tokyo is experiencing light rain and a temperature of 18°C."},
    }

    if city_normalized in mock_weather_db:
        return mock_weather_db[city_normalized]
    else:
        return {"status": "error", "error_message": f"Sorry, I don't have weather information for '{city}'."}

# # Example tool usage (optional test)
# print(get_weather("New York"))
# print(get_weather("Paris"))
```

### ADK agent

ADK [supports various models providers](https://google.github.io/adk-docs/agents/models/) via the [LiteLLM](https://docs.litellm.ai/docs/providers) package. 

Make sure to add the API keys for the models you want to use to your .env file. See `env.example` for reference.



```python
# @title Define the Weather Agent

# Select the provider and model you want to use
model_name = "gpt-5-nano" # see https://docs.litellm.ai/docs/providers/openai
# model_name = "claude-sonnet-4-20250514" # see https://docs.litellm.ai/docs/providers/anthropic
# model_name = "gemini-2.5-flash"


if model_name.startswith("gpt"):
    assert os.getenv("OPENAI_API_KEY"), "Please set the OPENAI_API_KEY environment variable."
    model = LiteLlm(model=model_name)  # Use LiteLlm for OpenAI or Anthropic models
    print(f"Using OpenAI model: {model_name}")
elif model_name.startswith("claude"):
    assert os.getenv("ANTHROPIC_API_KEY"), "Please set the ANTHROPIC_API_KEY environment variable."
    model = LiteLlm(model=model_name)  # Use LiteLlm for OpenAI or Anthropic models
    print(f"Using Anthropic model: {model_name}")
elif model_name.startswith("gemini"):
    assert os.getenv("GOOGLE_API_KEY"), "Please set the GOOGLE_API_KEY environment variable."
    model = model_name  # Use string for Gemini models
    print(f"Using Google model: {model_name}")
else:
    raise ValueError("Unsupported model. Please choose a valid model name.")

weather_agent = LlmAgent(
    name="weather_agent_v1",
    model=model,
    description="Provides weather information for specific cities.",
    instruction="You are a helpful weather assistant. "
                "When the user asks for the weather in a specific city, "
                "use the 'get_weather' tool to find the information. "
                "If the tool returns an error, inform the user politely. "
                "If the tool is successful, present the weather report clearly.",
    tools=[get_weather],
)
```

    Using OpenAI model: gpt-5-nano


## Running ADK with CAPSEM

To run ADK with CAPSEM, we need to:
- create a `SecurityManager` with the desired security policies (e.g., `DebugPolicy` for logging all actions).
- Initialize the ADK `CAPSEMPlugin` that acts as a bridge between CAPSEM and ADK with the `SecurityManager`.
- Use the `CAPSEMRunner` to run the ADK agent with CAPSEM support in order to be able to block unsafe actions. This step will hopefully be replaced with a native ADK support in the future.


### Instantiate CAPSEM security manager as ADK plugin


```python
security_manager = SecurityManager()
security_manager.add_policy(DebugPolicy())
capsem_plugin = CAPSEMPlugin(security_manager=security_manager)
```

### Agent Runner

#### Secure Runner
We use the `CAPSEMRunner` instead of ADK standard Runner to ensure ADK blocks unsafe actions. This is simply done by replacing the `Runner` class with our custom `CAPSEMRunner` that inherits from `Runner` which overrides the `async_run` method to check CAPSEM decisions before executing any action.


#### CAPSEM Plugin
Make sure to include the capsem plugin in the list of plugins when instantiating the runner otherwise the CAPSEM policy manager won't run.


```python
session_service = InMemorySessionService()
APP_NAME = "CAPSEM_ADK_Demo"
runner = CAPSEMRunner(agent=weather_agent, app_name=APP_NAME,
    session_service=session_service,
    plugins=[capsem_plugin]  # Integrate CAPSEM plugin
)

```

### Test run
Let's run now the agent with a simple prompt and with a prompt containing an unsafe request (containing the word "block") to demonstrate CAPSEM blocking unsafe actions.


```python
async def run_adk(prompt: str):
    USER_ID = "3713"
    SESSION_ID = f"s{uuid4().hex}"
    print("prompt:", prompt)
    # Create the specific session where the conversation will happen
    await session_service.create_session(app_name=APP_NAME,
                                         user_id=USER_ID,
                                         session_id=SESSION_ID)
    content = types.Content(role='user', parts=[types.Part(text=prompt)])
    final_response_text = "Agent did not produce a final response." # Default

    async for event in runner.run_async(user_id=USER_ID, session_id=SESSION_ID,
                                        new_message=content):
        # Key Concept: is_final_response() marks the concluding message for the turn.
        if event.is_final_response():
            if event.content and event.content.parts:
                # Assuming text response in the first part
                final_response_text = event.content.parts[0].text
            elif event.actions and event.actions.escalate: # Handle potential errors/escalations
                final_response_text = f"Agent escalated: {event.error_message or 'No specific message.'}"
            break # Stop processing events once the final response is found

    print(final_response_text)
```


```python
print("-=normal workflow=-")
query = "What's the weather like in new york?"
await run_adk(query)

# test blocking with DebugPolicy
print("\n-= Blocking workflow =-")
blocking_query = "block What's the weather like in new york?"
await run_adk(blocking_query)

```

    -=normal workflow=-
    prompt: What's the weather like in new york?
    
    [Decision][e5e3b4fad6b4][ALLOW][on_model_call] safe: 1 policies check passed.
    --- Tool: get_weather called for city: New York ---
    [Decision][40587cf1f2b8][ALLOW][on_tool_response] safe: 1 policies check passed.
    Current weather in New York: sunny with a temperature of 25°C.
    
    Would you like a forecast or want this converted to Fahrenheit, or more details (humidity, wind, etc.)?
    [Decision][bf6809d4ac7c][ALLOW][on_model_call] safe: 1 policies check passed.
    Current weather in New York: sunny with a temperature of 25°C.
    
    Would you like a forecast or want this converted to Fahrenheit, or more details (humidity, wind, etc.)?
    
    -= Blocking workflow =-
    prompt: block What's the weather like in new york?
    [Decision][738b19c5e0b1][BLOCK][on_model_call] policy_violation: Detected 'block' in prompt
    Agent did not produce a final response.

