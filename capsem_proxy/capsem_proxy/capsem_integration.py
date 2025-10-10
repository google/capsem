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

"""CAPSEM integration for the proxy"""

import os
import logging
from pathlib import Path

from capsem.manager import SecurityManager
from capsem.policies.debug_policy import DebugPolicy
from capsem.models import Agent, Decision, Verdict
from capsem.tools import Tool

logger = logging.getLogger(__name__)

# Initialize SecurityManager based on config directory or default to DebugPolicy
config_dir = os.environ.get("PROXY_CONFIG_DIR")

if config_dir and Path(config_dir).exists():
    logger.info(f"Loading policies from config directory: {config_dir}")
    try:
        from capsem.config_loader import load_policies_from_directory
        security_manager = load_policies_from_directory(config_dir)
        logger.info("Successfully loaded policies from config directory")
    except Exception as e:
        logger.error(f"Failed to load config: {e}")
        logger.warning("Falling back to default DebugPolicy")
        security_manager = SecurityManager()
        security_manager.add_policy(DebugPolicy())
else:
    if config_dir:
        logger.warning(f"Config directory not found: {config_dir}, using default DebugPolicy")
    else:
        logger.info("No config directory specified, using default DebugPolicy")
    security_manager = SecurityManager()
    security_manager.add_policy(DebugPolicy())


def create_agent(user_id: str, tools: list[dict] = None) -> Agent:
    """
    Create an Agent instance for CAPSEM from proxy context.

    Args:
        user_id: Hashed user identifier
        tools: List of tool definitions from OpenAI request

    Returns:
        Agent instance
    """
    capsem_tools = []
    if tools:
        for tool_def in tools:
            # Convert OpenAI tool format to CAPSEM Tool
            if tool_def.get("type") == "function":
                func = tool_def.get("function", {})
                # CAPSEM Tool requires non-empty description
                description = func.get("description", "") or "No description provided"
                capsem_tool = Tool(
                    name=func.get("name", "unknown"),
                    description=description,
                    parameters=func.get("parameters", {"type": "object", "properties": {}})
                )
                capsem_tools.append(capsem_tool)

    return Agent(
        name=f"proxy-user-{user_id}",
        instructions="OpenAI API proxy user",
        tools=capsem_tools
    )
