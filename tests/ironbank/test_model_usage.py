"""Independent usage expectations from original provider response fields."""

import json

import pytest
from ironbank.model_ledger import _usage_from_upstream
from ironbank.model_pricing import estimate_cost_usd


@pytest.mark.parametrize("wrapped", [False, True])
@pytest.mark.parametrize("streaming", [False, True])
def test_google_output_includes_separately_reported_thinking(wrapped, streaming):
    payload = {"usageMetadata": {
        "promptTokenCount": 31, "candidatesTokenCount": 17,
        "thoughtsTokenCount": 2, "totalTokenCount": 50,
    }}
    encoded = json.dumps({"response": payload} if wrapped else payload)
    row = {"response_body": f"data: {encoded}\n\n" if streaming else encoded,
           "content_type": "text/event-stream" if streaming else "application/json"}
    assert _usage_from_upstream(row) == {
        "input_tokens": 31, "output_tokens": 19, "thinking_tokens": 2,
    }


def test_google_tool_prompt_and_thinking_only_output_are_separate_components():
    row = {"response_body": json.dumps({"usageMetadata": {
        "promptTokenCount": 31, "toolUsePromptTokenCount": 4,
        "thoughtsTokenCount": 2, "totalTokenCount": 37,
    }})}
    assert _usage_from_upstream(row) == {
        "input_tokens": 35, "output_tokens": 2, "thinking_tokens": 2,
    }


def test_google_latest_usage_survives_a_final_chunk_without_usage():
    payload = {"usageMetadata": {"promptTokenCount": 31, "candidatesTokenCount": 17,
                                "thoughtsTokenCount": 2}}
    row = {"content_type": "text/event-stream", "response_body":
           f'data: {json.dumps({"response": payload})}\n\n'
           'data: {"response":{"candidates":[]}}\n\n'}
    assert _usage_from_upstream(row) == {
        "input_tokens": 31, "output_tokens": 19, "thinking_tokens": 2,
    }


@pytest.mark.parametrize("usage", [
    {"prompt_tokens": 31, "completion_tokens": 19,
     "completion_tokens_details": {"reasoning_tokens": 2}},
    {"input_tokens": 31, "output_tokens": 19,
     "output_tokens_details": {"reasoning_tokens": 2}},
    {"input_tokens": 31, "output_tokens": 19, "thinking_tokens": 2},
])
def test_inclusive_provider_output_does_not_add_reasoning_twice(usage):
    assert _usage_from_upstream({"response_body": json.dumps({"usage": usage})}) == {
        "input_tokens": 31, "output_tokens": 19, "thinking_tokens": 2,
    }


def test_internal_google_model_without_a_verified_rate_stays_unknown():
    assert estimate_cost_usd(provider="google", model="gemini-3.5-flash-low",
                             input_tokens=31, output_tokens=19,
                             usage_details={"thinking": 2},
                             timestamp="2026-10-23T22:45:35Z") is None
