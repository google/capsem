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

"Debug policy that prints all events and decisions"
from re import DEBUG
from ..tools import Tool
from .policy import Policy
from ..models import Agent, Media
from ..models import Decision, DEFAULT_SAFE_DECISION, Reason, Verdict

class DebugPolicy(Policy):
    "A policy that prints all events and returns safe decisions"

    DEBUG_TRIGGER = "capsem_block"

    def __init__(self):
        super().__init__(name="Debug",
                         description="Policy used to debug CAPSEM integration. Blocks any occurrence of the word 'capsem_block' in prompts, model responses, tool names, tool arguments, or tool responses.",
                         authors="Elie Bursztein",
                         url="https://github.com/google/capsem/policies/debug")

    async def on_workflow_start(self,
                                invocation_id: str,
                                agent: Agent,
                                prompt: str,
                                media: list[Media]) -> Decision:
        # not working as expected in ADK for now
        return DEFAULT_SAFE_DECISION

    async def on_workflow_end(self, invocation_id: str,
                              agent: Agent) -> Decision:
        return DEFAULT_SAFE_DECISION

    async def on_agent_start(self, invocation_id: str,
                             agent: Agent) -> Decision:
        return DEFAULT_SAFE_DECISION

    async def on_agent_end(self, invocation_id: str, agent: Agent) -> Decision:
        return DEFAULT_SAFE_DECISION

    async def on_tool_call(self, invocation_id: str, agent: Agent, tool: Tool, args: dict) -> Decision:
        if self.DEBUG_TRIGGER in tool.name.lower():
            return Decision(verdict=Verdict.BLOCK,
                            reason=Reason.POLICY_VIOLATION,
                            details=f"Detected '{self.DEBUG_TRIGGER}' in tool name")

        args_str = ", ".join(f"{k}={v}" for k, v in args.items())
        if self.DEBUG_TRIGGER in args_str.lower():
            return Decision(verdict=Verdict.BLOCK,
                            reason=Reason.POLICY_VIOLATION,
                            details=f"Detected '{self.DEBUG_TRIGGER}' in tool arguments")

        return DEFAULT_SAFE_DECISION

    async def on_tool_response(self, invocation_id: str, agent: Agent,
                               tool: Tool, response: dict) -> Decision:

        response_str = str(response)
        if 'capsem_block' in response_str.lower():
            return Decision(verdict=Verdict.BLOCK,
                            reason=Reason.POLICY_VIOLATION,
                            details=f"Detected '{self.DEBUG_TRIGGER}' in tool response")

        return DEFAULT_SAFE_DECISION

    async def on_model_call(self, invocation_id: str, agent: Agent,
                            model_name: str, system_instructions: str,
                            prompt: str, media: list[Media]) -> Decision:
        if "capsem_block" in prompt.lower():
            decision = Decision(verdict=Verdict.BLOCK,
                            reason=Reason.POLICY_VIOLATION,
                            details=f"Detected '{self.DEBUG_TRIGGER}' in prompt")
        else:
            decision = DEFAULT_SAFE_DECISION
        return decision

    async def on_model_response(self,
                                invocation_id: str,
                                agent: Agent, response: str,
                                thoughts: str, media: list[Media]) -> Decision:
        if 'capsem_block' in response.lower():
            return Decision(verdict=Verdict.BLOCK,
                            reason=Reason.POLICY_VIOLATION,
                            details=f"Detected '{self.DEBUG_TRIGGER}' in model response")

        return DEFAULT_SAFE_DECISION

    @classmethod
    def from_config(cls, config: dict) -> "DebugPolicy":
        """Create DebugPolicy from configuration dictionary

        Args:
            config: Configuration dictionary with optional keys:
                - enabled: bool (if False, returns None to skip policy)

        Returns:
            DebugPolicy instance, or None if disabled

        Example:
            config = {"enabled": True}
            policy = DebugPolicy.from_config(config)
        """
        # Check if policy is disabled
        if not config.get("enabled", True):
            return None

        return cls()