from __future__ import annotations

from tests.ironbank.test_model_client_ledger_contract import (
    ModelClientEnv,
    test_openai_two_tool_calls_have_exact_item_cardinality as _run_exact_ledger_check,
)


def test_openai_two_tool_calls_have_exact_item_cardinality(
    model_client_env: ModelClientEnv,
) -> None:
    _run_exact_ledger_check(model_client_env)
