"""Ironbank two-turn model/tool/file ledger proof."""

from __future__ import annotations

from typing import Any

from ironbank.model_client_assertions import assert_two_turn_model_client
from ironbank.model_client_scripts import openai_two_tool_calls_script


def test_two_turn_model_ledger_exact_cardinality(model_client_env: Any):
    result = assert_two_turn_model_client(
        model_client_env,
        openai_two_tool_calls_script("https://api.openai.com"),
    )
    assert result["provider"] == "openai"
    assert result["domain"] == "api.openai.com"
    assert result["path"] == "/v1/responses"
    assert len(result["results"]) == 2
