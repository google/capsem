"""Reviewed missing upstream prices survive refresh without overriding upstream."""

from __future__ import annotations

import copy
import runpy
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = runpy.run_path(str(ROOT / "build_system/scripts/build/update_genai_prices.py"))


def supplement() -> dict:
    return {
        "verified_on": "2026-10-23",
        "models": [
            {
                "provider": "google",
                "source_url": "https://ai.google.dev/gemini-api/docs/pricing",
                "model": {
                    "id": "gemini-test",
                    "match": {"equals": "gemini-test"},
                    "prices": {"input_mtok": 2, "output_mtok": 10},
                },
            }
        ],
    }


def test_supplement_adds_only_missing_exact_ids_and_upstream_always_wins() -> None:
    upstream = [{"id": "google", "models": []}]
    merged = SCRIPT["merge_reviewed_prices"](upstream, supplement())
    assert merged[0]["models"][0]["prices"]["input_mtok"] == 2
    upstream[0]["models"][0]["prices"]["input_mtok"] = 3
    SCRIPT["merge_reviewed_prices"](upstream, supplement())
    assert upstream[0]["models"][0]["prices"]["input_mtok"] == 3


@pytest.mark.parametrize("mutation", ["broad_match", "no_source", "duplicate", "unknown_provider"])
def test_reviewed_prices_refuse_unproven_aliases_and_missing_provenance(mutation: str) -> None:
    data = supplement()
    row = data["models"][0]
    if mutation == "broad_match":
        row["model"]["match"] = {"starts_with": "gemini-"}
    elif mutation == "no_source":
        row["source_url"] = ""
    elif mutation == "duplicate":
        data["models"].append(copy.deepcopy(row))
    else:
        row["provider"] = "unreviewed"
    with pytest.raises(ValueError):
        SCRIPT["merge_reviewed_prices"]([{"id": "google", "models": []}], data)
