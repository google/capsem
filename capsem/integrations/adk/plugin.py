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

from google.adk.agents import LlmAgent, BaseAgent
from google.adk.tools import BaseTool, ToolContext, FunctionTool
from google.adk.agents.callback_context import CallbackContext
from google.adk.agents.invocation_context import InvocationContext
from google.adk.models.llm_request import LlmRequest
from google.adk.models.llm_response import LlmResponse
from google.adk.plugins.base_plugin import BasePlugin
from google.adk.events import Event
from google.adk.sessions import Session
from google.genai import types
from typing import Any, Optional
import warnings

from ...tools import Tool
from ...models import Media, Agent, Decision, Verdict, Reason
from ...manager import SecurityManager


class CAPSEMPlugin(BasePlugin):
    """A custom adk plugin that counts agent and tool invocations."""

    def __init__(self,
                 security_manager: SecurityManager,
                 ):
        name = "CAPSEM"
        super().__init__(name=name)
        self.name = name
        self.security_manager = security_manager

    async def _parse_parts(self,
                     parts: list[types.Part] | None
    ) -> tuple[str, str, list[Media]]:
        "Convert ADK parts to CAPSEM prompt, thoughts, and media"
        prompt_parts = []
        thoughts_parts = []
        media: list[Media] = []

        if parts and len(parts) > 0:
            for part in parts:
                if part.text:
                    if part.thought:
                        thoughts_parts.append(part.text)
                    else:
                        prompt_parts.append(part.text)
                elif part.inline_data:
                    if part.inline_data.data and part.inline_data.mime_type:
                        m = Media(
                            content=part.inline_data.data,
                            mime_type=part.inline_data.mime_type,
                        )
                        media.append(m)

        # merge parts
        prompt = "\n".join(prompt_parts)
        thoughts = "\n".join(thoughts_parts)

        return prompt, thoughts, media

    async def _parse_adk_agent(self, adkagent: BaseAgent) -> Agent:
        "Convert ADK agent to CAPSEM agent name"

        if not adkagent:
            raise ValueError("Agent is None")

        # some agents like sequential agents do not have instructions or tools
        cagent = Agent(name=adkagent.name, instructions="")

        if isinstance(adkagent, LlmAgent):
            # include instructions
            if isinstance(adkagent.instruction, str):
                cagent.instructions = adkagent.instruction
            else:
                raise ValueError("Agent instructions is not a string")

            # add tools
            for at in adkagent.tools:
                # gymnastics to convert ADK tool to CAPSEM tool
                ft = FunctionTool(at)
                d = ft._get_declaration()
                if not d:
                    continue
                t = Tool.from_json(d.model_dump_json())
                cagent.tools.append(t)

            # recursively parse sub agents
            for sa in adkagent.sub_agents:
                sat = self._parse_adk_agent(sa)
                cagent.sub_agents.append(sat)

        return cagent

    async def _set_decision(self,
                      context: CallbackContext | InvocationContext | ToolContext,
                      decision: Decision):
        """Set the decision in the state

        Notes: due to the async nature of the callbacks, we need to check
        if the decision in memory to avoid race conditions. Sadly we cannot
        mutate the state from the runner directly so that's the best we can do.

        """
        if not isinstance(decision, Decision):
            raise ValueError("decision is not a Decision instance")

        state = None
        if isinstance(context, InvocationContext):
            state = context.session.state
        elif isinstance(context, CallbackContext):
            state = context.state
        elif isinstance(context, ToolContext):
            state = context.state
        else:
            warnings(f"Unknown context type {type(context)}, cannot set decision")

        if not state:
            return

        past_decision = state.get("capsem")
        if isinstance(past_decision, Decision):
            if past_decision.verdict.value == Verdict.BLOCK.value:
                # once blocked, always blocked
                return
        # if no previous decision or not blocked, set the new decision
        state["capsem"] = decision.model_dump()
        # print('[plugin]current decision:', state)

    async def on_user_message_callback(
        self,
        *,
        invocation_context: InvocationContext,
        user_message: types.Content):

        invocation_id = invocation_context.invocation_id
        cagent = await self._parse_adk_agent(invocation_context.agent)

        # normalize parts
        prompt, thoughts, media = await self._parse_parts(user_message.parts)
        decision = await self.security_manager.on_workflow_start(invocation_id,
                                                                  cagent,
                                                                  prompt,
                                                                  media)
        await self._set_decision(invocation_context, decision)

    async def after_run_callback(
        self, *, invocation_context: InvocationContext
    ) -> Optional[None]:
        """Callback executed after an ADK runner run has completed."""
        cagent = await self._parse_adk_agent(invocation_context.agent)
        invocation_id = invocation_context.invocation_id
        decision = await self.security_manager.on_workflow_end(invocation_id,
                                                                cagent)
        # await self._set_decision(invocation_context, decision)

    async def before_agent_callback(self, *,
                                    agent: BaseAgent,
                                    callback_context: CallbackContext
                                  ) -> Optional[types.Content]:
        """Callback executed before an agent's primary logic is invoked."""
        invocation_id = callback_context.invocation_id
        cagent = await self._parse_adk_agent(agent)
        decision = await self.security_manager.on_agent_start(invocation_id,
                                                              cagent)
        # await self._set_decision(callback_context, decision)

    async def after_agent_callback(
        self, *, agent: BaseAgent, callback_context: CallbackContext
    ) -> Optional[types.Content]:
        """Callback executed after an agent's primary logic has completed.
        """
        invocation_id = callback_context.invocation_id
        cagent = await self._parse_adk_agent(agent)
        decision = await self.security_manager.on_agent_end(invocation_id,
                                                             cagent)
        # await self._set_decision(callback_context, decision)

    async def before_model_callback(self, *,
                                    callback_context: CallbackContext,
                                    llm_request: LlmRequest
                                    ) -> Optional[LlmResponse]:
        """Callback executed before a request is sent to the model."""
        invocation_id = callback_context.invocation_id
        cagent = await self._parse_adk_agent(callback_context._invocation_context.agent)
        prompt, thoughts, media = await self._parse_parts(llm_request.contents[0].parts)
        model_name = llm_request.model or "unknown"
        insructions = str(llm_request.config.system_instruction) or ""
        decision = await self.security_manager.on_model_call(invocation_id,
                                                             cagent, model_name,
                                                             insructions,
                                                             prompt, media)

        await self._set_decision(callback_context, decision)



        if decision.verdict.name == Verdict.BLOCK.name:
            # If CAPSEM decided to block, we skip the model call
            # and return an empty response.
            text = f"Model call blocked by security policy {decision.policy} - reason: {decision.reason.name}"
            return LlmResponse(
                content=types.Content(parts=[types.Part(text=text)])
            )


    async def after_model_callback(self, *, callback_context: CallbackContext,
                                   llm_response: LlmResponse) -> Optional[LlmResponse]:
        """Callback executed after a response is received from the model."""
        invocation_id = callback_context.invocation_id
        cagent = await self._parse_adk_agent(callback_context._invocation_context.agent)

        # parse response
        if llm_response.content:
            response, thoughts, media = await self._parse_parts(llm_response.content.parts)
        else:
            response, thoughts, media = "", "", []

        decision = await self.security_manager.on_model_response(invocation_id,
                                                                 cagent,
                                                                 response,
                                                                 thoughts,
                                                                 media)
        await self._set_decision(callback_context, decision)
        if decision.verdict.name == Verdict.BLOCK.name:
            # If CAPSEM decided to block, we skip the model call
            # and return an empty response.
            text = f"{decision.verdict}: {decision.reason}: {decision.details}"
            return LlmResponse(content=types.Content(parts=[types.Part(text=text)]))


    async def before_tool_callback(
        self,
        *,
        tool: BaseTool,
        tool_args: dict[str, Any],
        tool_context: ToolContext,
    ) -> Optional[dict]:
        """Callback executed before a tool is called."""
        invocation_id = tool_context._invocation_context.invocation_id
        cagent = await self._parse_adk_agent(tool_context._invocation_context.agent)
        d = tool._get_declaration()
        if not d:
            raise ValueError("Tool does not have a declaration")
        ctool = Tool.from_json(d.model_dump_json())
        decision = await self.security_manager.on_tool_call(invocation_id,
                                                            cagent, ctool,
                                                            tool_args)
        await self._set_decision(tool_context, decision)

        if decision.verdict.name == Verdict.BLOCK.name:
            # If CAPSEM decided to block, we skip the tool call
            # and return an empty result.
            return {}

    async def after_tool_callback(self, *, tool: BaseTool,
                                  tool_args: dict[str, Any],
                                  tool_context: ToolContext,
                                  result: dict):
        """Callback executed after a tool has been called."""

        invocation_id = tool_context._invocation_context.invocation_id
        cagent = await self._parse_adk_agent(tool_context._invocation_context.agent)
        d = tool._get_declaration()
        if not d:
            raise ValueError("Tool does not have a declaration")
        ctool = Tool.from_json(d.model_dump_json())
        decision = await self.security_manager.on_tool_response(invocation_id,
                                                                cagent, ctool,
                                                                result)
        await self._set_decision(tool_context, decision)

        if decision.verdict.name == Verdict.BLOCK.name:
            # If CAPSEM decided to block, we skip the tool result
            # and return an empty result.
            return {}
