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

from uuid import uuid4
from pydantic import BaseModel, Field
from enum import Enum
from .tools import Tool

class Agent(BaseModel):
    "abstract representation of an agent"
    name: str = Field(description="Name of the agent")
    instructions: str = Field(description="Instructions for the agent")
    tools: list[Tool] = Field(default_factory=list,
                              description="List of tools available to the agent")
    sub_agents: list['Agent'] = Field(default_factory=list,
                                      description="List of sub-agents")


class Media(BaseModel):
    "Media associated with a prompt"
    content: bytes = Field(description="Raw content of the media")
    mime_type: str = Field(description="MIME type of the media e.g., image/png")

class Verdict(Enum):
    "Possible verdicts for a policy evaluation"
    ALLOW = "allow" # allow the agent to proceed
    BLOCK = "block" # block the agent execution
    CONFIRM = "confirm"  # ask the user for confirmation

class Reason(str, Enum):
    "Possible reasons for a policy decision"

    SAFE = "safe"                 # default safe catch-all
    # default violation catch-all
    POLICY_VIOLATION = "policy_violation" # the agent is violating a defined policy

    # specific reasons
    SENSITIVE_DATA = "sensitive_data" # the agent is trying to access sensitive data
    HIGH_RISK_ACTION = "high_risk_action" # the agent is trying to perform a high-risk action
    UNKNOWN_TOOL = "unknown_tool" # the agent is trying to use an unknown tool
    EXCESSIVE_USAGE = "excessive_usage" # the agent is using resources excessively
    PROMPT_INJECTION = "prompt_injection" # the agent's prompt contains potential injection attacks
    LEAKAGE = "leakage" # the agent is leaking sensitive information
    MALICIOUS_BEHAVIOR = "malicious_behavior" # the agent is exhibiting malicious
    UNSAFE_CONTENT = "unsafe_content" # the agent is generating unsafe content

class Decision(BaseModel):
    "A decision made by a policy"
    id : str = Field(default_factory=lambda: uuid4().hex[:12],
                     description="Unique identifier for the decision")
    policy: str = Field(default="unknown",
                        description="Name of the policy that made the decision")
    callback: str = Field(default="unknown",
                          description="Name of the callback that triggered the decision")
    verdict: Verdict = Field(default=Verdict.ALLOW,
                             description="The verdict of the policy")
    reason: Reason = Field(default=Reason.POLICY_VIOLATION,
                        description="Standardized reason for the decision")
    details: str = Field(default="",
                         description="Additional details about the decision")

    def __str__(self) -> str:
        return f"[Decision][{self.id}][{self.verdict.value.upper()}][{self.callback}] {self.reason.value}: {self.details}"

# syntactic sugar for common decisions
DEFAULT_SAFE_DECISION = Decision(verdict=Verdict.ALLOW, reason=Reason.SAFE)
DEFAULT_BLOCK_DECISION = Decision(verdict=Verdict.BLOCK, reason=Reason.POLICY_VIOLATION)
DEFAULT_CONFIRM_DECISION = Decision(verdict=Verdict.CONFIRM, reason=Reason.POLICY_VIOLATION)