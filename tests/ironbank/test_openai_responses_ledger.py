"""OpenAI Responses API Ironbank release gate.

This is the provider-specific S02-020 entrypoint. The shared model-client
fixture owns the service, VM, mock-server, DB, routes, and logs; these tests
keep the OpenAI release surface executable by name.
"""

from __future__ import annotations

import pytest

from ironbank.model_client_assertions import assert_one_model_client
from ironbank.model_client_scripts import openai_responses_api_script
from tests.ironbank.test_model_client_ledger_contract import (
    ModelClientEnv,
    _assert_openai_embeddings_and_image_ledger,
)

pytestmark = pytest.mark.integration


def test_openai_responses_streaming_tool_embeddings_and_image_ledger(
    model_client_env: ModelClientEnv,
) -> None:
    result = assert_one_model_client(
        model_client_env,
        openai_responses_api_script("https://api.openai.com"),
    )
    assert result["provider"] == "openai"
    assert result["credential_provider"] == "openai"
    assert result["domain"] == "api.openai.com"
    assert result["path"] == "/v1/responses"
    assert result["tool_call_name"] == "exec_command"
    assert result["file_matches"] is True

    _assert_openai_embeddings_and_image_ledger(model_client_env)


def test_openai_responses_two_turn_exact_cardinality(
    model_client_env: ModelClientEnv,
) -> None:
    from tests.ironbank.test_model_client_ledger_contract import (
        test_openai_two_tool_calls_have_exact_item_cardinality,
    )

    test_openai_two_tool_calls_have_exact_item_cardinality(model_client_env)
