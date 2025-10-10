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

from tabulate import tabulate

from .tools import Tool
from .policies import Policy
from .models import Decision, Media, Agent, Verdict, Reason

class SecurityManager:
    def __init__(self):
        self._policies: dict[str, Policy] = {}

    # Policy management
    def add_policy(self, policy: Policy):
        "Add a policy to the manager"
        if policy.name in self._policies:
            raise ValueError(f"Policy {policy.name} already exists")
        self._policies[policy.name] = policy

    def get_policies(self) -> list[Policy]:
        "Get the list of policies"
        return list(self._policies.values())

    def print_policies(self):
        "Print the list of policies"
        table = [[p.name] for p in self.get_policies()]
        print(tabulate(table, headers=["Name"]))

    # policies callbacks

    ## Workflow
    async def on_workflow_start(self,
                                invocation_id: str,
                                agent: Agent,
                                prompt: str, media: list[Media]) -> Decision:
        "Called when an agent workflow starts"
        decisions = {}
        for name, policy in self._policies.items():
            decisions[name] = await policy.on_workflow_start(invocation_id,
                                                             agent,
                                                             prompt, media)
        return self._decide(decisions, 'on_workflow_start')

    async def on_workflow_end(self, invocation_id: str, agent: Agent) -> Decision:
        "Called when an agent workflow ends"
        decisions = {}
        for name, policy in self._policies.items():
            decisions[name] = await policy.on_workflow_end(invocation_id, agent)
        return self._decide(decisions, 'on_workflow_end')

    # Agent
    async def on_agent_start(self, invocation_id: str, agent: Agent) -> Decision:
        "Called when an agent starts"
        decisions = {}
        for name, policy in self._policies.items():
            decisions[name] = await policy.on_agent_start(invocation_id,
                                                          agent)
        return self._decide(decisions, 'on_agent_start')

    async def on_agent_end(self, invocation_id: str, agent: Agent) -> Decision:
        "Called when an agent ends"
        decisions = {}
        for name, policy in self._policies.items():
            decisions[name] = await policy.on_agent_end(invocation_id, agent)
        return self._decide(decisions, 'on_agent_end')

    # Tool
    async def on_tool_call(self, invocation_id: str, agent: Agent, tool: Tool, args: dict) -> Decision:
        "Called when an agent is calling a tool"
        decisions = {}
        for name, policy in self._policies.items():
            decisions[name] = await policy.on_tool_call(invocation_id,
                                                        agent, tool, args)
        return self._decide(decisions, 'on_tool_call')

    async def on_tool_response(self, invocation_id: str, agent: Agent,
                               tool: Tool, response: dict) -> Decision:
        "Called when an agent receives a tool response"
        decisions = {}
        for name, policy in self._policies.items():
            decisions[name] = await policy.on_tool_response(invocation_id,
                                                            agent, tool,
                                                            response)
        return self._decide(decisions, 'on_tool_response')

    # Model
    async def on_model_call(self, invocation_id: str,
                            agent: Agent,
                            model_name: str,
                            system_instructions: str,
                            prompt: str, media: list[Media]) -> Decision:
        "Called when an agent is calling a model"
        decisions = {}
        for name, policy in self._policies.items():
            decisions[name] = await policy.on_model_call(invocation_id,
                                                         agent,
                                                         model_name,
                                                         system_instructions,
                                                         prompt,
                                                         media)
        decision = self._decide(decisions, 'on_model_call')
        return decision

    async def on_model_response(self,
                                invocation_id,
                                agent: Agent,
                                response: str,
                                thoughts: str,
                                media: list[Media]) -> Decision:
        "Called when an agent receives a model response"
        decisions = {}
        for name, policy in self._policies.items():
            decisions[name] = await policy.on_model_response(invocation_id,
                                                             agent,
                                                             response,
                                                             thoughts,
                                                             media)
        return self._decide(decisions, 'on_model_response')

    def _decide(self, decisions: dict[str, Decision], callback: str) -> Decision:
        "Combine all decisions into a single one by following the strictest one"

        # if any policy blocks, we block
        for pname, decision in decisions.items():
            # !keep the .value comparison to avoid enum identity issues
            if decision.verdict.value == Verdict.BLOCK.value:
                return Decision(
                    policy=pname,
                    callback=callback,
                    verdict=Verdict.BLOCK,
                    reason=decision.reason,
                    details=decision.details
                )

        # if any policy asks for confirmation, we ask for confirmation
        for pname, decision in decisions.items():
            # !keep the .value comparison to avoid enum identity issues
            if decision.verdict.value == Verdict.CONFIRM.value:
                return Decision(
                    policy=pname,
                    callback=callback,
                    verdict=Verdict.CONFIRM,
                    reason=decision.reason,
                    details=decision.details
                )

        # otherwise, we allow
        return Decision(policy="capsem",
                        callback=callback,
                        verdict=Verdict.ALLOW,
                        reason=Reason.SAFE,
                        details=f"{len(decisions)} policies check passed.")
