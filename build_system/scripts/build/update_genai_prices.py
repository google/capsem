#!/usr/bin/env python3
"""Transform pydantic/genai-prices into Capsem's compact runtime ledger."""

from __future__ import annotations

import argparse
import json
from datetime import date
from pathlib import Path
from typing import Any

RUNTIME_PROVIDERS = {"anthropic", "google", "openai"}
PROVIDER_FIELDS = ("id", "name", "pricing_urls", "api_pattern", "models")
MODEL_FIELDS = ("id", "match", "context_window", "prices")
MODEL_ID_PREFIXES = {
    "anthropic": ("claude-",),
    "google": ("gemini-", "gemini_", "gemma-", "gemma_"),
    "openai": (
        "chatgpt-",
        "codex-",
        "computer-use",
        "dall-e",
        "ft:",
        "gpt-",
        "gpt.",
        "gpt_",
        "o1",
        "o2",
        "o3",
        "o4",
        "omni-",
        "text-",
        "tts-",
        "whisper-",
    ),
}


def merge_reviewed_prices(
    providers: list[dict[str, Any]], reviewed: dict[str, Any]
) -> list[dict[str, Any]]:
    """Fill exact missing IDs from reviewed primary sources; upstream wins on refresh."""
    date.fromisoformat(reviewed["verified_on"])
    seen: set[tuple[str, str]] = set()
    for row in reviewed["models"]:
        provider_id = row["provider"]
        model = row["model"]
        identity = (provider_id, model["id"])
        if (
            provider_id not in RUNTIME_PROVIDERS
            or identity in seen
            or not row["source_url"].startswith("https://")
            or model["match"] != {"equals": model["id"]}
        ):
            raise ValueError(
                "reviewed prices require unique exact IDs and primary-source provenance"
            )
        seen.add(identity)
        aliases = row.get("aliases", [])
        if aliases and (
            not isinstance(aliases, list)
            or not row.get("alias_note")
            or any(not isinstance(alias, str) or not alias.startswith(model["id"] + "-") for alias in aliases)
            or len(set(aliases)) != len(aliases)
        ):
            raise ValueError("reviewed aliases require exact distinct family IDs and a review note")
        provider = next((entry for entry in providers if entry["id"] == provider_id), None)
        if provider is None:
            raise ValueError(f"missing upstream provider: {provider_id}")
        if not any(entry["id"] == model["id"] for entry in provider["models"]):
            provider["models"].append({key: model[key] for key in MODEL_FIELDS if key in model})
        if aliases:
            canonical = next(entry for entry in provider["models"] if entry["id"] == model["id"])
            existing = canonical["match"].get("or", [canonical["match"]])
            canonical["match"] = {"or": existing + [{"equals": alias} for alias in aliases if {"equals": alias} not in existing]}
    return providers


def compact_pricing(data: list[dict[str, Any]]) -> list[dict[str, Any]]:
    providers: list[dict[str, Any]] = []
    for provider in data:
        provider_id = provider.get("id")
        if provider_id not in RUNTIME_PROVIDERS:
            continue
        compact_provider = {
            key: provider[key] for key in PROVIDER_FIELDS if key in provider and key != "models"
        }
        models = []
        for model in provider.get("models") or []:
            if not isinstance(model, dict):
                continue
            model_id = str(model.get("id") or "")
            if not model_id.startswith(MODEL_ID_PREFIXES[provider_id]):
                continue
            if "match" not in model or "prices" not in model:
                continue
            models.append({key: model[key] for key in MODEL_FIELDS if key in model})
        compact_provider["models"] = models
        providers.append(compact_provider)
    providers.sort(key=lambda item: item["id"])
    return providers


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("dest", type=Path)
    parser.add_argument(
        "--reviewed",
        type=Path,
        default=Path(__file__).resolve().parents[3] / "config/data/genai-prices-reviewed.json",
    )
    args = parser.parse_args()

    data = json.loads(args.source.read_text(encoding="utf-8"))
    if not isinstance(data, list):
        raise SystemExit("upstream pricing data must be a JSON list")
    compact = compact_pricing(data)
    ids = {provider["id"] for provider in compact}
    if ids != RUNTIME_PROVIDERS:
        raise SystemExit(f"missing runtime providers: {sorted(RUNTIME_PROVIDERS - ids)}")
    merge_reviewed_prices(compact, json.loads(args.reviewed.read_text(encoding="utf-8")))
    args.dest.parent.mkdir(parents=True, exist_ok=True)
    args.dest.write_text(
        json.dumps(compact, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
