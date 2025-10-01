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

from networkx import Graph

from ..tools import Tool
from ..models import Media, Agent
from ..models import Decision, DEFAULT_SAFE_DECISION


class Policy():
    "Defines a Security and Privacy Policy"
    def __init__(self,
                 name: str,
                 description: str,
                 authors: str,
                 url: str):
        """Initialize the policy aheads of agents executions
        Policys must be re-entrable and stateless as they may be called
        concurrently by multiples agents executions.

        Args:
            name (str): Name of the policy
            description (str): Description of the policy
            authors (str): Authors of the policy
            url (str): URL describing the policy e.g research paper or page.

        Notes:
            The description and url are for documentation purposes and used to
            generate the documentation website.
        """

        self.name = name
        self.description = description
        self.url = url
        self.authors = authors

    async def on_workflow_start(self,
                                invocation_id: str,
                                agent: Agent,
                                prompt: str,
                                media: list[Media]) -> Decision:
        "Called when the agent workflow starts"
        return DEFAULT_SAFE_DECISION

    async def on_workflow_end(self,
                              invocation_id: str,
                              agent: Agent) -> Decision:
        "Called when the agent workflow ends"
        return DEFAULT_SAFE_DECISION

    async def on_agent_start(self,
                             invocation_id: str,
                             agent: Agent) -> Decision:
        "Called when an agent starts"
        return DEFAULT_SAFE_DECISION

    async def on_agent_end(self,
                           invocation_id: str,
                           agent: Agent) -> Decision:
        "Called when an agent ends"
        return DEFAULT_SAFE_DECISION

    async def on_tool_call(self,
                           invocation_id: str,
                           agent: Agent,
                           tool: Tool,
                           args: dict) -> Decision:
        "Called when the agent is calling a tool"
        return DEFAULT_SAFE_DECISION

    async def on_tool_response(self,
                               invocation_id: str,
                               agent: Agent,
                               tool: Tool,
                               response: dict) -> Decision:
        "Called when the agent receives a tool response"
        return DEFAULT_SAFE_DECISION

    async def on_model_call(self,
                            invocation_id: str,
                            agent: Agent,
                            model_name: str,
                            system_instructions: str,
                            prompt: str,
                            media: list[Media]) -> Decision:
        "Called when the agent is calling a model"
        return DEFAULT_SAFE_DECISION

    async def on_model_response(self,
                                invocation_id: str,
                                agent: Agent,
                                response: str,
                                thoughts: str,
                                media: list[Media]) -> Decision:
        "Called when the agent receives a model response"
        return DEFAULT_SAFE_DECISION
