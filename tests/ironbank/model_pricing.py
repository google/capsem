"""Pricing oracle for Ironbank model ledger checks."""

from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parents[2]
PRICING_PATH = PROJECT_ROOT / "config" / "data" / "genai-prices.json"
PRICE_EPSILON_USD = 1e-10


def assert_model_call_price(row: Any) -> None:
    """Assert DB model_call cost matches the bundled pricing ledger."""

    usage_details = json.loads(row["usage_details"] or "{}")
    expected = estimate_cost_usd(
        provider=row["provider"],
        model=row["model"],
        input_tokens=int(row["input_tokens"] or 0),
        output_tokens=int(row["output_tokens"] or 0),
        usage_details=usage_details,
    )
    actual = float(row["estimated_cost_usd"] or 0.0)
    assert abs(actual - expected) <= PRICE_EPSILON_USD, {
        "provider": row["provider"],
        "model": row["model"],
        "input_tokens": row["input_tokens"],
        "output_tokens": row["output_tokens"],
        "usage_details": usage_details,
        "actual_estimated_cost_usd": actual,
        "expected_estimated_cost_usd": expected,
    }
    if has_pricing(provider=row["provider"], model=row["model"]):
        token_total = int(row["input_tokens"] or 0) + int(row["output_tokens"] or 0)
        if token_total > 0:
            assert actual > 0.0, {
                "provider": row["provider"],
                "model": row["model"],
                "input_tokens": row["input_tokens"],
                "output_tokens": row["output_tokens"],
                "estimated_cost_usd": actual,
            }


def estimate_cost_usd(
    *,
    provider: str,
    model: str | None,
    input_tokens: int,
    output_tokens: int,
    usage_details: dict[str, Any],
) -> float:
    model_str = model or ""
    if not model_str or len(model_str) > 128:
        return 0.0
    if provider not in {"anthropic", "openai", "google"}:
        return 0.0
    effective_input = max(0, input_tokens - int(usage_details.get("cache_read") or 0))
    if effective_input == 0 and output_tokens == 0:
        return 0.0
    provider_data = _provider(provider)
    if provider_data is None:
        return 0.0
    price = _strict_price(provider_data, model_str)
    if price is None:
        price = _suffix_stripped_price(provider_data, model_str)
    if price is None:
        price = _prefix_price(provider_data, model_str)
    if price is None:
        return 0.0
    return (
        effective_input * _rate(price.get("input_mtok")) / 1_000_000.0
        + output_tokens * _rate(price.get("output_mtok")) / 1_000_000.0
    )


def has_pricing(*, provider: str, model: str | None) -> bool:
    model_str = model or ""
    if not model_str or len(model_str) > 128 or provider not in {"anthropic", "openai", "google"}:
        return False
    provider_data = _provider(provider)
    if provider_data is None:
        return False
    return (
        _strict_price(provider_data, model_str)
        or _suffix_stripped_price(provider_data, model_str)
        or _prefix_price(provider_data, model_str)
    ) is not None


@lru_cache(maxsize=1)
def _pricing_data() -> list[dict[str, Any]]:
    return json.loads(PRICING_PATH.read_text(encoding="utf-8"))


def _provider(provider: str) -> dict[str, Any] | None:
    return next((entry for entry in _pricing_data() if entry.get("id") == provider), None)


def _strict_price(provider_data: dict[str, Any], model: str) -> dict[str, Any] | None:
    for entry in provider_data.get("models") or []:
        if _matches(entry.get("match") or {}, model):
            return _price(entry)
    return None


def _suffix_stripped_price(provider_data: dict[str, Any], model: str) -> dict[str, Any] | None:
    candidate = model
    for _ in range(4):
        pos = candidate.rfind("-")
        if pos < 4:
            break
        candidate = candidate[:pos]
        price = _strict_price(provider_data, candidate)
        if price is not None:
            return price
    return None


def _prefix_price(provider_data: dict[str, Any], model: str) -> dict[str, Any] | None:
    best_entry: dict[str, Any] | None = None
    best_len = 0
    best_version: int | None = None
    for entry in provider_data.get("models") or []:
        model_id = str(entry.get("id") or "")
        prefix_len = _common_prefix_len(model, model_id)
        if prefix_len < 8:
            continue
        version = _trailing_version(model_id)
        if prefix_len > best_len or (
            prefix_len == best_len and version is not None and (best_version is None or version > best_version)
        ):
            best_entry = entry
            best_len = prefix_len
            best_version = version
    return _price(best_entry) if best_entry is not None else None


def _matches(rule: dict[str, Any], model: str) -> bool:
    if "equals" in rule:
        return model == rule["equals"]
    if "starts_with" in rule:
        return model.startswith(rule["starts_with"])
    if "ends_with" in rule:
        return model.endswith(rule["ends_with"])
    if "contains" in rule:
        return rule["contains"] in model
    if "or" in rule:
        return any(_matches(option, model) for option in rule["or"])
    return False


def _price(entry: dict[str, Any] | None) -> dict[str, Any] | None:
    if entry is None:
        return None
    prices = entry.get("prices")
    if isinstance(prices, dict):
        return prices
    if isinstance(prices, list) and prices:
        first = prices[0]
        nested = first.get("prices") if isinstance(first, dict) else None
        if isinstance(nested, dict):
            return nested
    return None


def _rate(value: Any) -> float:
    if isinstance(value, int | float):
        return float(value)
    if isinstance(value, dict):
        return float(value.get("base") or 0.0)
    return 0.0


def _common_prefix_len(a: str, b: str) -> int:
    count = 0
    for left, right in zip(a.encode(), b.encode(), strict=False):
        if left != right:
            break
        count += 1
    return count


def _trailing_version(model_id: str) -> int | None:
    segment = model_id.rsplit("-", 1)[-1]
    try:
        return int(segment)
    except ValueError:
        return None
