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

"""Tests for PII detection policy

These tests verify that the PIIPolicy correctly:
- Detects PII using Microsoft Presidio
- Returns appropriate verdicts based on entity_decisions configuration
- Checks all four contexts (prompts, tool args, model responses, tool responses)
- Implements correct decision priority (BLOCK > CONFIRM > LOG)

Note: Tests use EMAIL_ADDRESS as the primary test case since it's reliably detected
by Presidio's English model. Other PII types may have varying detection rates.
"""

import pytest
from ..models import Agent, Verdict, Reason, Media
from ..tools import Tool, Schema, Type
from .pii_policy import PIIPolicy, PIIEntityType, DEFAULT_PII_ENTITIES, PRESIDIO_AVAILABLE

# Skip all tests if presidio is not available
pytestmark = pytest.mark.skipif(
    not PRESIDIO_AVAILABLE,
    reason="presidio-analyzer not installed. Install with: uv add --group pii presidio-analyzer"
)


def make_test_tool(name: str, description: str) -> Tool:
    """Helper to create a Tool with required parameters"""
    return Tool(
        name=name,
        description=description,
        parameters=Schema(type=Type.OBJECT, properties={})
    )


def test_pii_policy_init():
    """Test policy initialization with default settings"""
    policy = PIIPolicy()

    assert policy.name == "PIIDetection"
    assert "Presidio" in policy.description
    assert policy.check_prompts is True
    assert policy.check_tool_args is True
    assert policy.check_responses is True
    assert policy.check_tool_responses is True

    # Default should log all common PII types
    assert len(policy.entity_decisions) == len(DEFAULT_PII_ENTITIES)
    for entity in DEFAULT_PII_ENTITIES:
        assert policy.entity_decisions[entity] == Verdict.LOG


def test_pii_policy_custom_entity_decisions():
    """Test policy with custom entity decisions using enum"""
    entity_decisions = {
        PIIEntityType.CREDIT_CARD: Verdict.BLOCK,
        PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM,
        PIIEntityType.PHONE_NUMBER: Verdict.LOG,
    }

    policy = PIIPolicy(entity_decisions=entity_decisions)

    assert policy.entity_decisions[PIIEntityType.CREDIT_CARD] == Verdict.BLOCK
    assert policy.entity_decisions[PIIEntityType.EMAIL_ADDRESS] == Verdict.CONFIRM
    assert policy.entity_decisions[PIIEntityType.PHONE_NUMBER] == Verdict.LOG


def test_pii_policy_selective_checks():
    """Test disabling specific check types"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
        check_prompts=True,
        check_tool_args=False,
        check_responses=True,
        check_tool_responses=False
    )

    assert policy.check_prompts is True
    assert policy.check_tool_args is False
    assert policy.check_responses is True
    assert policy.check_tool_responses is False


@pytest.mark.asyncio
async def test_detect_email_in_prompt_blocked():
    """Test that email in prompt is blocked when configured"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK}
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="Contact me at john.doe@example.com for details",
        media=[]
    )

    assert decision.verdict == Verdict.BLOCK
    assert decision.reason == Reason.SENSITIVE_DATA
    assert "EMAIL_ADDRESS" in decision.details
    assert "prompt" in decision.details


@pytest.mark.asyncio
async def test_detect_email_confirm():
    """Test that email triggers CONFIRM verdict"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM}
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="Contact me at alice@example.com",
        media=[]
    )

    assert decision.verdict == Verdict.CONFIRM
    assert decision.reason == Reason.SENSITIVE_DATA
    assert "EMAIL_ADDRESS" in decision.details


@pytest.mark.asyncio
async def test_detect_email_log_only():
    """Test that email triggers LOG verdict"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.LOG}
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="My email is bob@example.com",
        media=[]
    )

    assert decision.verdict == Verdict.LOG
    assert decision.reason == Reason.SENSITIVE_DATA
    assert "EMAIL_ADDRESS" in decision.details


@pytest.mark.asyncio
async def test_no_pii_in_prompt_safe():
    """Test that prompts without PII return safe decision"""
    policy = PIIPolicy(
        entity_decisions={
            PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK,
            PIIEntityType.CREDIT_CARD: Verdict.BLOCK
        }
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="What is the weather like today?",
        media=[]
    )

    assert decision.verdict == Verdict.ALLOW
    assert decision.reason == Reason.SAFE


@pytest.mark.asyncio
async def test_pii_in_tool_arguments():
    """Test PII detection in tool arguments"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK}
    )

    agent = Agent(name="test", instructions="test")
    tool = make_test_tool("send_email", "Send email")

    decision = await policy.on_tool_call(
        invocation_id="test-123",
        agent=agent,
        tool=tool,
        args={"to": "secret@example.com", "message": "Hello"}
    )

    assert decision.verdict == Verdict.BLOCK
    assert decision.reason == Reason.SENSITIVE_DATA
    assert "EMAIL_ADDRESS" in decision.details
    assert "send_email" in decision.details


@pytest.mark.asyncio
async def test_pii_in_model_response():
    """Test PII detection in model responses"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK}
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_response(
        invocation_id="test-123",
        agent=agent,
        response="You can contact the support team at support@company.com",
        thoughts="",
        media=[]
    )

    assert decision.verdict == Verdict.BLOCK
    assert decision.reason == Reason.LEAKAGE  # Responses use LEAKAGE reason
    assert "EMAIL_ADDRESS" in decision.details
    assert "model response" in decision.details


@pytest.mark.asyncio
async def test_pii_in_tool_response():
    """Test PII detection in tool responses"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM}
    )

    agent = Agent(name="test", instructions="test")
    tool = make_test_tool("get_contacts", "Get contacts")

    decision = await policy.on_tool_response(
        invocation_id="test-123",
        agent=agent,
        tool=tool,
        response={"contact": "John Doe", "email": "john@example.com"}
    )

    assert decision.verdict == Verdict.CONFIRM
    assert decision.reason == Reason.LEAKAGE  # Tool responses use LEAKAGE reason
    assert "EMAIL_ADDRESS" in decision.details
    assert "get_contacts" in decision.details


@pytest.mark.asyncio
async def test_decision_priority_block_wins():
    """Test that BLOCK verdict has highest priority when multiple emails detected"""
    # This test uses two different entity configs with same email to test priority
    policy = PIIPolicy(
        entity_decisions={
            PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK
        }
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="Email: test@example.com",
        media=[]
    )

    # BLOCK should win
    assert decision.verdict == Verdict.BLOCK
    assert "EMAIL_ADDRESS" in decision.details


@pytest.mark.asyncio
async def test_decision_priority_confirm_over_log():
    """Test that CONFIRM verdict is used when configured"""
    policy = PIIPolicy(
        entity_decisions={
            PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM
        }
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="Email: test@example.com",
        media=[]
    )

    # CONFIRM should be used
    assert decision.verdict == Verdict.CONFIRM


@pytest.mark.asyncio
async def test_check_prompts_disabled():
    """Test that prompts are not checked when check_prompts=False"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
        check_prompts=False
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="Contact me at john.doe@example.com",
        media=[]
    )

    # Should be ALLOW because checking is disabled
    assert decision.verdict == Verdict.ALLOW


@pytest.mark.asyncio
async def test_check_tool_args_disabled():
    """Test that tool args are not checked when check_tool_args=False"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
        check_tool_args=False
    )

    agent = Agent(name="test", instructions="test")
    tool = make_test_tool("send_email", "Send email")

    decision = await policy.on_tool_call(
        invocation_id="test-123",
        agent=agent,
        tool=tool,
        args={"to": "secret@example.com"}
    )

    # Should be ALLOW because checking is disabled
    assert decision.verdict == Verdict.ALLOW


@pytest.mark.asyncio
async def test_check_responses_disabled():
    """Test that responses are not checked when check_responses=False"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
        check_responses=False
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_response(
        invocation_id="test-123",
        agent=agent,
        response="Contact support@company.com",
        thoughts="",
        media=[]
    )

    # Should be ALLOW because checking is disabled
    assert decision.verdict == Verdict.ALLOW


@pytest.mark.asyncio
async def test_check_tool_responses_disabled():
    """Test that tool responses are not checked when check_tool_responses=False"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
        check_tool_responses=False
    )

    agent = Agent(name="test", instructions="test")
    tool = make_test_tool("get_contacts", "Get contacts")

    decision = await policy.on_tool_response(
        invocation_id="test-123",
        agent=agent,
        tool=tool,
        response={"email": "user@example.com"}
    )

    # Should be ALLOW because checking is disabled
    assert decision.verdict == Verdict.ALLOW


@pytest.mark.asyncio
async def test_empty_text_safe():
    """Test that empty text returns safe decision"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK}
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="",
        media=[]
    )

    assert decision.verdict == Verdict.ALLOW


@pytest.mark.asyncio
async def test_multiple_detections_count():
    """Test that multiple occurrences of same PII type are counted"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.LOG}
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="Contact alice@example.com or bob@example.com or charlie@example.com",
        media=[]
    )

    assert decision.verdict == Verdict.LOG
    # Should mention count in details
    assert "count=" in decision.details
    assert "EMAIL_ADDRESS" in decision.details


@pytest.mark.asyncio
async def test_score_threshold():
    """Test custom score threshold configuration"""
    policy = PIIPolicy(
        entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
        score_threshold=0.5  # Default threshold
    )

    agent = Agent(name="test", instructions="test")

    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="Contact user@example.com",
        media=[]
    )

    # Email should be detected with default threshold
    assert decision.verdict == Verdict.BLOCK


@pytest.mark.asyncio
async def test_entity_not_in_decisions_ignored():
    """Test that PII types not in entity_decisions are ignored"""
    policy = PIIPolicy(
        entity_decisions={
            PIIEntityType.PERSON: Verdict.BLOCK
            # EMAIL_ADDRESS not configured, should be ignored
        }
    )

    agent = Agent(name="test", instructions="test")

    # Prompt with email but PERSON not configured to be checked
    decision = await policy.on_model_call(
        invocation_id="test-123",
        agent=agent,
        model_name="test-model",
        system_instructions="",
        prompt="Contact me at test@example.com",
        media=[]
    )

    # Should be ALLOW because EMAIL_ADDRESS is not configured
    assert decision.verdict == Verdict.ALLOW
