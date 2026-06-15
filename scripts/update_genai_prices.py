#!/usr/bin/env python3
"""Transform pydantic/genai-prices into Capsem's compact runtime ledger."""

from __future__ import annotations

import argparse
import json
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
    args = parser.parse_args()

    data = json.loads(args.source.read_text(encoding="utf-8"))
    if not isinstance(data, list):
        raise SystemExit("upstream pricing data must be a JSON list")
    compact = compact_pricing(data)
    ids = {provider["id"] for provider in compact}
    if ids != RUNTIME_PROVIDERS:
        raise SystemExit(f"missing runtime providers: {sorted(RUNTIME_PROVIDERS - ids)}")
    args.dest.parent.mkdir(parents=True, exist_ok=True)
    args.dest.write_text(
        json.dumps(compact, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
