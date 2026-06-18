"""Optional live-provider canaries for model ledger compatibility.

These tests are compatibility diagnostics, not release proof. They run only
when an operator explicitly provides provider credentials in the environment or
in the configured live-provider dotenv file. Hermetic replay tests remain the
release gate.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Callable

import pytest

from ironbank.model_client_assertions import assert_live_model_client
from ironbank.model_client_scripts import (
    live_claude_messages_script,
    live_gemini_generate_content_script,
    live_openai_chat_completions_script,
    live_openai_responses_api_script,
)
from tests.ironbank.test_model_client_ledger_contract import (
    ModelClientEnv,
    _credential_ref_for_secret,
    _live_provider_secret,
    live_model_client_env,  # noqa: F401 - imported to register the pytest fixture.
)


@dataclass(frozen=True)
class LiveProviderCanary:
    id: str
    provider: str
    env_keys: tuple[str, ...]
    script: Callable[[], str]
    domain: str
    path: str | None
    expected_model_calls: int = 2


LIVE_PROVIDER_CANARIES = (
    LiveProviderCanary(
        id="openai_chat_completions",
        provider="openai",
        env_keys=("OPENAI_API_KEY",),
        script=live_openai_chat_completions_script,
        domain="api.openai.com",
        path="/v1/chat/completions",
    ),
    LiveProviderCanary(
        id="openai_responses",
        provider="openai",
        env_keys=("OPENAI_API_KEY",),
        script=live_openai_responses_api_script,
        domain="api.openai.com",
        path="/v1/responses",
    ),
    LiveProviderCanary(
        id="gemini_generate_content",
        provider="google",
        env_keys=("GEMINI_API_KEY", "GOOGLE_API_KEY"),
        script=live_gemini_generate_content_script,
        domain="generativelanguage.googleapis.com",
        path=None,
    ),
    LiveProviderCanary(
        id="claude_messages",
        provider="anthropic",
        env_keys=("ANTHROPIC_API_KEY",),
        script=live_claude_messages_script,
        domain="api.anthropic.com",
        path="/v1/messages",
    ),
)

TRACKED_MANUAL_LIVE_CANARIES = {
    "agy": "AGY OAuth live proof is tracked by S02-016 because it needs an interactive OAuth dance, not an env-key canary.",
}


def test_live_provider_canary_matrix_is_explicit() -> None:
    ids = {canary.id for canary in LIVE_PROVIDER_CANARIES}
    assert ids == {
        "openai_chat_completions",
        "openai_responses",
        "gemini_generate_content",
        "claude_messages",
    }
    assert {canary.provider for canary in LIVE_PROVIDER_CANARIES} == {
        "openai",
        "google",
        "anthropic",
    }
    assert TRACKED_MANUAL_LIVE_CANARIES == {
        "agy": "AGY OAuth live proof is tracked by S02-016 because it needs an interactive OAuth dance, not an env-key canary.",
    }


def test_live_provider_scripts_are_provider_specific() -> None:
    rendered = {canary.id: canary.script() for canary in LIVE_PROVIDER_CANARIES}
    assert "/v1/chat/completions" in rendered["openai_chat_completions"]
    assert "/v1/responses" in rendered["openai_responses"]
    assert "generativelanguage.googleapis.com" in rendered["gemini_generate_content"]
    assert ":generateContent" in rendered["gemini_generate_content"]
    assert "api.anthropic.com" in rendered["claude_messages"]
    assert "/v1/messages" in rendered["claude_messages"]


@pytest.mark.live_provider
@pytest.mark.parametrize(
    "canary",
    LIVE_PROVIDER_CANARIES,
    ids=[canary.id for canary in LIVE_PROVIDER_CANARIES],
)
def test_optional_live_provider_canary_pays_ledger_debt(
    request: pytest.FixtureRequest,
    canary: LiveProviderCanary,
) -> None:
    secret = _first_available_secret(canary.env_keys)
    if secret is None:
        pytest.skip(
            f"{' or '.join(canary.env_keys)} not provided for optional live-provider canary"
        )
    env: ModelClientEnv = request.getfixturevalue("live_model_client_env")
    result = assert_live_model_client(
        env,
        canary.script(),
        raw_secret=secret,
        expected_credential_ref=_credential_ref_for_secret(secret, provider=canary.provider),
        expected_model_calls=canary.expected_model_calls,
        timeout_secs=240,
    )
    assert result["provider"] == canary.provider
    assert result["domain"] == canary.domain
    if canary.path is not None:
        assert result["path"] == canary.path


def _first_available_secret(env_keys: tuple[str, ...]) -> str | None:
    for key in env_keys:
        if secret := _live_provider_secret(key):
            return secret
    return None
