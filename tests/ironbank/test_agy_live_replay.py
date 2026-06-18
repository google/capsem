"""AGY/Antigravity release replay gate.

This is the S02-023 provider entrypoint. Live AGY OAuth is intentionally
manual; the release proof is the hermetic replay through the same Capsem
network/model/credential ledger path.
"""

from __future__ import annotations

import pytest

from ironbank.model_client_assertions import assert_one_model_client
from ironbank.model_client_config import HERMETIC_AGY_MODEL, HERMETIC_AGY_MODEL_DISPLAY
from ironbank.model_client_scripts import agy_cli_script
from tests.ironbank.test_live_provider_canaries import TRACKED_MANUAL_LIVE_CANARIES
from tests.ironbank.test_model_client_ledger_contract import ModelClientEnv

pytestmark = pytest.mark.integration


def test_agy_cloudcode_replay_pays_release_ledger_debt(
    model_client_env: ModelClientEnv,
) -> None:
    result = assert_one_model_client(
        model_client_env,
        agy_cli_script(model_client_env.mock_base_url),
    )

    assert result["provider"] == "google"
    assert result["credential_provider"] == "google"
    assert result["credential_source"] == "http.header.authorization"
    assert result["domain"] == "daily-cloudcode-pa.googleapis.com"
    assert result["path"] == "/v1internal:streamGenerateContent"
    assert result["model"] == HERMETIC_AGY_MODEL
    assert result["tool_call_name"] == "run_command"
    assert result["file_matches"] is True


def test_agy_release_model_and_manual_live_oauth_contract_are_explicit() -> None:
    assert HERMETIC_AGY_MODEL == "gemini-3.5-flash-low"
    assert HERMETIC_AGY_MODEL_DISPLAY == "Gemini 3.5 Flash (Medium)"
    assert TRACKED_MANUAL_LIVE_CANARIES == {
        "agy": "AGY OAuth live proof is tracked by S02-016 because it needs an interactive OAuth dance, not an env-key canary.",
    }
