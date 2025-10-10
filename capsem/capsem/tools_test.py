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

import json
from .tools import Tool, ToolSet
from google.genai import types as genai_types

# taken from various external documentation sources
SPECS = {
    # https://ai.google.dev/gemini-api/docs/function-calling?example=weather
    "weather": {
        "name": "get_current_temperature",
        "description": "Gets the current temperature for a given location.",
        "parameters": {
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city name, e.g. San Francisco",
                },
            },
        "required": ["location"],
        },
    },
    # https://ai.google.dev/gemini-api/docs/function-calling?example=meeting
    "meeting": {
        "name": "schedule_meeting",
        "description": "Schedules a meeting with specified attendees at a given time and date.",
        "parameters": {
            "type": "object",
            "properties": {
                "attendees": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of people attending the meeting.",
                },
                "date": {
                    "type": "string",
                    "description": "Date of the meeting (e.g., '2024-07-29')",
                },
                "time": {
                    "type": "string",
                    "description": "Time of the meeting (e.g., '15:00')",
                },
                "topic": {
                    "type": "string",
                    "description": "The subject or topic of the meeting.",
                },
            },
            "required": ["attendees", "date", "time", "topic"],
        },
    }

}

def test_indepotence():
    "check our tool representations preserve all information"
    for spec_name, spec in SPECS.items():
        tool = Tool.from_dict(spec)
        tool_json = tool.to_json()
        tool_dict = json.loads(tool_json)
        for key in spec:
            assert key in tool_dict
            assert tool_dict[key] == spec[key]

def test_gemini_compatibility():
    "check our tools works with Gemini function calling"
    tool = Tool.from_dict(SPECS["meeting"])
    tool_dict = tool.to_dict()
    print(tool_dict)
    fn = genai_types.FunctionDeclaration(**tool_dict)
    genai_types.Tool(function_declarations=[fn])
