"""Ironbank pricing oracle contract tests."""

from __future__ import annotations

import pytest
from ironbank.model_pricing import assert_model_call_price, estimate_cost_usd, has_pricing


def test_openai_gpt5_nano_fixture_price_is_exact() -> None:
    assert has_pricing(provider="openai", model="gpt-5-nano")
    assert estimate_cost_usd(
        provider="openai",
        model="gpt-5-nano",
        input_tokens=1000,
        output_tokens=250,
        usage_details={},
    ) == pytest.approx(0.00015)


def test_cache_read_tokens_are_not_charged_as_full_input() -> None:
    assert estimate_cost_usd(
        provider="openai",
        model="gpt-5-nano",
        input_tokens=1000,
        output_tokens=0,
        usage_details={"cache_read": 400},
    ) == pytest.approx(0.000032)


def test_claude_sonnet_46_tiered_base_price_matches_product_rule() -> None:
    assert has_pricing(provider="anthropic", model="claude-sonnet-4-6")
    assert estimate_cost_usd(
        provider="anthropic",
        model="claude-sonnet-4-6",
        input_tokens=1000,
        output_tokens=100,
        usage_details={},
    ) == pytest.approx(0.0045)


def test_pricing_uses_upstream_match_without_suffix_guessing() -> None:
    assert has_pricing(provider="openai", model="gpt-5-nano")
    assert not has_pricing(provider="openai", model="gpt-5-private-fork")
    assert (
        estimate_cost_usd(
            provider="openai",
            model="gpt-5-private-fork",
            input_tokens=1000,
            output_tokens=250,
            usage_details={},
        )
        is None
    )


def test_conditional_prices_use_the_recorded_utc_date() -> None:
    values = {
        "provider": "openai",
        "model": "o3",
        "input_tokens": 1_000_000,
        "output_tokens": 1_000_000,
        "usage_details": {},
    }
    assert estimate_cost_usd(**values, timestamp="2025-06-09T23:59:59Z") == 50.0
    assert estimate_cost_usd(**values, timestamp="2025-06-10T00:00:00Z") == 10.0


def test_prompt_tiers_charge_entire_call_and_cache_write_has_its_own_rate() -> None:
    assert estimate_cost_usd(
        provider="google",
        model="gemini-1.5-pro",
        input_tokens=128001,
        output_tokens=1_000_000,
        usage_details={},
    ) == pytest.approx(10.3200025)
    assert estimate_cost_usd(
        provider="anthropic",
        model="claude-sonnet-4-6",
        input_tokens=1000,
        output_tokens=0,
        usage_details={"cache_write": 400},
    ) == pytest.approx(0.0033)


def test_unknown_null_is_not_an_observed_zero_and_missing_usage_stays_unknown() -> None:
    row = {
        "provider": "google",
        "model": "gemini-4-argon-trial",
        "input_tokens": 100,
        "output_tokens": 10,
        "usage_details": "{}",
        "estimated_cost_usd": None,
        "timestamp": "2026-10-23T00:00:00Z",
    }
    assert_model_call_price(row)
    with pytest.raises(AssertionError):
        assert_model_call_price({**row, "estimated_cost_usd": 0.0})
    assert (
        estimate_cost_usd(
            provider="openai",
            model="gpt-5-nano",
            input_tokens=None,
            output_tokens=5,
            usage_details={},
        )
        is None
    )


def test_known_measured_zero_is_valid() -> None:
    assert_model_call_price(
        {
            "provider": "openai",
            "model": "gpt-5-nano",
            "input_tokens": 0,
            "output_tokens": 0,
            "usage_details": "{}",
            "estimated_cost_usd": 0.0,
            "timestamp": "2026-10-23T00:00:00Z",
        }
    )
