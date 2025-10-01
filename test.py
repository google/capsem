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

from pydantic import BaseModel, Field
from typing import List, Dict, Any, Optional, Union
import json

class ParameterProperty(BaseModel):
    type: str
    description: Optional[str] = None
    items: Optional[Dict[str, Any]] = None  # For arrays
    enum: Optional[List[str]] = None

class ParameterSchema(BaseModel):
    type: str = "object"
    properties: Dict[str, ParameterProperty]
    required: Optional[List[str]] = None

class ReturnSchema(BaseModel):
    type: str
    description: Optional[str] = None
    properties: Optional[Dict[str, ParameterProperty]] = None

class Tool(BaseModel):
    name: str = Field(..., description="The name of the tool/function")
    description: str = Field(..., description="Description of what the tool does")
    parameters: ParameterSchema = Field(..., description="Parsed parameter schema")
    returns: Optional[ReturnSchema] = Field(None, description="Return type schema")
    
    @staticmethod
    def from_json(json_data: Dict[str, Any]) -> 'Tool':
        """
        Create a Tool instance from a JSON dictionary (OpenAI function format).
        """
        # Parse parameters properly
        params_data = json_data["parameters"]
        properties = {}
        for prop_name, prop_data in params_data.get("properties", {}).items():
            properties[prop_name] = ParameterProperty(**prop_data)
        
        parameters = ParameterSchema(
            type=params_data.get("type", "object"),
            properties=properties,
            required=params_data.get("required", [])
        )
        
        # Parse return schema if present
        returns = None
        if "returns" in json_data:
            return_data = json_data["returns"]
            return_props = {}
            if "properties" in return_data:
                for prop_name, prop_data in return_data["properties"].items():
                    return_props[prop_name] = ParameterProperty(**prop_data)
            
            returns = ReturnSchema(
                type=return_data.get("type", "object"),
                description=return_data.get("description"),
                properties=return_props if return_props else None
            )
        
        return Tool(
            name=json_data["name"],
            description=json_data["description"],
            parameters=parameters,
            returns=returns
        )
    
    @staticmethod
    def from_mcp(mcp_data: Dict[str, Any]) -> 'Tool':
        """
        Create a Tool instance from MCP (Model Context Protocol) format.
        Properly parses the inputSchema instead of just copying it.
        """
        # Parse inputSchema properly
        input_schema = mcp_data.get("inputSchema", {})
        properties = {}
        for prop_name, prop_data in input_schema.get("properties", {}).items():
            properties[prop_name] = ParameterProperty(**prop_data)
        
        parameters = ParameterSchema(
            type=input_schema.get("type", "object"),
            properties=properties,
            required=input_schema.get("required", [])
        )
        
        # Parse outputSchema if present
        returns = None
        if "outputSchema" in mcp_data:
            output_data = mcp_data["outputSchema"]
            output_props = {}
            if "properties" in output_data:
                for prop_name, prop_data in output_data["properties"].items():
                    output_props[prop_name] = ParameterProperty(**prop_data)
            
            returns = ReturnSchema(
                type=output_data.get("type", "object"),
                description=output_data.get("description"),
                properties=output_props if output_props else None
            )
        
        return Tool(
            name=mcp_data["name"],
            description=mcp_data.get("description", ""),
            parameters=parameters,
            returns=returns
        )
    
    def to_openai_format(self) -> Dict[str, Any]:
        """Convert Tool to OpenAI function calling format."""
        result = {
            "name": self.name,
            "description": self.description,
            "parameters": {
                "type": self.parameters.type,
                "properties": {
                    name: prop.model_dump(exclude_none=True) 
                    for name, prop in self.parameters.properties.items()
                },
            }
        }
        
        if self.parameters.required:
            result["parameters"]["required"] = self.parameters.required
            
        if self.returns:
            result["returns"] = self.returns.model_dump(exclude_none=True)
            
        return result
    
    def to_mcp_format(self) -> Dict[str, Any]:
        """Convert Tool to MCP format."""
        result = {
            "name": self.name,
            "description": self.description,
            "inputSchema": {
                "type": self.parameters.type,
                "properties": {
                    name: prop.model_dump(exclude_none=True) 
                    for name, prop in self.parameters.properties.items()
                },
            }
        }
        
        if self.parameters.required:
            result["inputSchema"]["required"] = self.parameters.required
            
        if self.returns:
            result["outputSchema"] = self.returns.model_dump(exclude_none=True)
            
        return result

# Example usage:
if __name__ == "__main__":
    # Your original schedule_meeting_function with return type added
    schedule_meeting_function = {
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
        "returns": {
            "type": "object",
            "description": "Meeting scheduling result",
            "properties": {
                "meeting_id": {
                    "type": "string",
                    "description": "Unique identifier for the scheduled meeting"
                },
                "status": {
                    "type": "string",
                    "description": "Status of the scheduling operation"
                }
            }
        }
    }
    
    # Create Tool from JSON - now properly parsing the schema
    tool = Tool.from_json(schedule_meeting_function)
    print("Tool created from JSON:")
    print(f"Name: {tool.name}")
    print(f"Parameters type: {tool.parameters.type}")
    print(f"Properties: {list(tool.parameters.properties.keys())}")
    print(f"Required: {tool.parameters.required}")
    if tool.returns:
        print(f"Returns: {tool.returns.type}")
        if tool.returns.properties:
            print(f"Return properties: {list(tool.returns.properties.keys())}")

