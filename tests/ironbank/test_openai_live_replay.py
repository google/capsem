"""OpenAI live-canary and replay gate wiring.

Live provider calls are optional diagnostics because they require operator
credentials. Hermetic replay remains the release proof and is exercised in
``test_openai_responses_ledger.py``.
"""

from __future__ import annotations

from ironbank.model_client_config import (
    LIVE_OPENAI_EMBEDDING_MODEL,
    LIVE_OPENAI_IMAGE_MODEL,
    LIVE_OPENAI_RESPONSES_MODEL,
)
from tests.ironbank.test_live_provider_canaries import LIVE_PROVIDER_CANARIES


def test_openai_live_canary_contracts_are_explicit() -> None:
    openai_canaries = {
        canary.id: canary for canary in LIVE_PROVIDER_CANARIES if canary.provider == "openai"
    }

    assert set(openai_canaries) == {"openai_chat_completions", "openai_responses"}
    assert openai_canaries["openai_chat_completions"].domain == "api.openai.com"
    assert openai_canaries["openai_chat_completions"].path == "/v1/chat/completions"
    assert openai_canaries["openai_responses"].domain == "api.openai.com"
    assert openai_canaries["openai_responses"].path == "/v1/responses"


def test_openai_release_models_are_centralized() -> None:
    assert LIVE_OPENAI_RESPONSES_MODEL == "gpt-5-nano"
    assert LIVE_OPENAI_IMAGE_MODEL == "gpt-5.5"
    assert LIVE_OPENAI_EMBEDDING_MODEL == "text-embedding-3-small"
