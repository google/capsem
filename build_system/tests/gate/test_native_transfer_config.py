"""Native transfer is bounded inside the existing install job and cache owner."""

from pathlib import Path

import pytest
from capsem_builder.gate.config import load
from pydantic import ValidationError

ROOT = Path(__file__).resolve().parents[3]


@pytest.mark.parametrize(
    "field,value",
    [
        ("stage", "test-evidence"),
        ("prefix", "foreign"),
        ("poll_seconds", float("inf")),
        ("hold_seconds", 0),
        ("hold_seconds", 600),
        ("outer_seconds", 900),
        ("base_hold_seconds", 14400),
        ("base_outer_seconds", 10000),
    ],
)
def test_transfer_cannot_select_an_unaccounted_owner_or_outlive_its_bound(field, value):
    selected = load(ROOT).functional.native_images.transfer
    with pytest.raises(ValidationError):
        type(selected).model_validate({**selected.model_dump(), field: value})
