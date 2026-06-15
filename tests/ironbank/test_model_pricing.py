"""Ironbank pricing oracle contract tests."""

from __future__ import annotations

import pytest

from ironbank.model_pricing import estimate_cost_usd, has_pricing


def test_openai_gpt5_nano_fixture_price_is_exact() -> None:
    assert has_pricing(provider="openai", model="gpt-5-nano")
    assert (
        estimate_cost_usd(
            provider="openai",
            model="gpt-5-nano",
            input_tokens=1000,
            output_tokens=250,
            usage_details={},
        )
        == pytest.approx(0.00015)
    )


def test_cache_read_tokens_are_not_charged_as_full_input() -> None:
    assert (
        estimate_cost_usd(
            provider="openai",
            model="gpt-5-nano",
            input_tokens=1000,
            output_tokens=0,
            usage_details={"cache_read": 400},
        )
        == pytest.approx(0.00003)
    )


def test_claude_sonnet_46_tiered_base_price_matches_product_rule() -> None:
    assert has_pricing(provider="anthropic", model="claude-sonnet-4-6")
    assert (
        estimate_cost_usd(
            provider="anthropic",
            model="claude-sonnet-4-6",
            input_tokens=1000,
            output_tokens=100,
            usage_details={},
        )
        == pytest.approx(0.0045)
    )
