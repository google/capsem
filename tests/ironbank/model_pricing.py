"""Pricing oracle for Ironbank model ledger checks."""

from __future__ import annotations

import json
import math
import re
from datetime import UTC, date, datetime
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
        input_tokens=row["input_tokens"],
        output_tokens=row["output_tokens"],
        usage_details=usage_details,
        timestamp=row["timestamp"],
    )
    actual = row["estimated_cost_usd"]
    if expected is None:
        assert actual is None, "unpriced usage must remain NULL, not a measured zero"
        return
    assert actual is not None and abs(float(actual) - expected) <= PRICE_EPSILON_USD, {
        "provider": row["provider"],
        "model": row["model"],
        "input_tokens": row["input_tokens"],
        "output_tokens": row["output_tokens"],
        "usage_details": usage_details,
        "actual_estimated_cost_usd": actual,
        "expected_estimated_cost_usd": expected,
    }


def estimate_cost_usd(
    *,
    provider: str,
    model: str | None,
    input_tokens: int | None,
    output_tokens: int | None,
    usage_details: dict[str, Any],
    timestamp: str | None = None,
) -> float | None:
    model_str = model or ""
    if not model_str or len(model_str) > 128:
        return None
    if provider not in {"anthropic", "openai", "google"}:
        return None
    if input_tokens is None or output_tokens is None or min(input_tokens, output_tokens) < 0:
        return None
    read = int(usage_details.get("cache_read") or 0)
    write = int(usage_details.get("cache_write") or 0)
    write_1h = int(usage_details.get("cache_write_1h") or 0)
    effective_input = input_tokens - read - write
    if min(effective_input, read, write, write_1h) < 0 or write_1h > write or (write_1h and provider != "anthropic"):
        return None
    provider_data = _provider(provider)
    if provider_data is None:
        return None
    price = _matched_price(provider_data, model_str, _day(timestamp))
    if price is None:
        return None
    total = 0.0
    for count, key in [
        (effective_input, "input_mtok"),
        (output_tokens, "output_mtok"),
        (read, "cache_read_mtok"),
        (write - write_1h, "cache_write_mtok"),
    ]:
        if count:
            rate = _rate(price.get(key), input_tokens)
            if rate is None:
                return None
            total += count * rate / 1_000_000.0
    if write_1h:
        rate = _rate(price.get("input_mtok"), input_tokens)
        if rate is None:
            return None
        total += write_1h * 2 * rate / 1_000_000.0
    return total


def has_pricing(*, provider: str, model: str | None) -> bool:
    model_str = model or ""
    if not model_str or len(model_str) > 128 or provider not in {"anthropic", "openai", "google"}:
        return False
    provider_data = _provider(provider)
    if provider_data is None:
        return False
    return _matched_price(provider_data, model_str, _day(None)) is not None


@lru_cache(maxsize=1)
def _pricing_data() -> list[dict[str, Any]]:
    return json.loads(PRICING_PATH.read_text(encoding="utf-8"))


def _provider(provider: str) -> dict[str, Any] | None:
    return next((entry for entry in _pricing_data() if entry.get("id") == provider), None)


def _day(timestamp: str | None) -> date:
    if timestamp is None:
        return datetime.now(UTC).date()
    parsed = datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    assert parsed.tzinfo is not None, "ledger pricing requires an explicit UTC offset"
    return parsed.astimezone(UTC).date()


def _matched_price(provider_data: dict[str, Any], model: str, day: date) -> dict[str, Any] | None:
    for entry in provider_data.get("models") or []:
        if _matches(entry.get("match") or {}, model):
            return _price(entry, day)
    return None


def _matches(rule: dict[str, Any], model: str) -> bool:
    model = model.lower()
    if "equals" in rule:
        return model == rule["equals"].lower()
    if "starts_with" in rule:
        return model.startswith(rule["starts_with"].lower())
    if "ends_with" in rule:
        return model.endswith(rule["ends_with"].lower())
    if "contains" in rule:
        return rule["contains"].lower() in model
    if "regex" in rule:
        return re.search(rule["regex"], model, re.IGNORECASE) is not None
    if "or" in rule:
        return any(_matches(option, model) for option in rule["or"])
    return False


def _price(entry: dict[str, Any] | None, day: date) -> dict[str, Any] | None:
    if entry is None:
        return None
    prices = entry.get("prices")
    if isinstance(prices, dict):
        return prices
    if isinstance(prices, list):
        for candidate in reversed(prices):
            constraint = candidate.get("constraint")
            if constraint is not None:
                assert set(constraint) == {"start_date"}, "unsupported pricing constraint"
                if date.fromisoformat(constraint["start_date"]) > day:
                    continue
            return candidate["prices"]
    return None


def _rate(value: Any, input_tokens: int) -> float | None:
    if isinstance(value, int | float):
        result = float(value)
    elif isinstance(value, dict):
        result = float(value["base"])
        for tier in sorted(value["tiers"], key=lambda tier: tier["start"]):
            if input_tokens > tier["start"]:
                result = float(tier["price"])
    else:
        return None
    return result if math.isfinite(result) and result >= 0 else None
