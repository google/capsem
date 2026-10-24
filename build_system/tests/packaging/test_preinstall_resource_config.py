"""Package-owned bounds are product budgets, not a universal dpkg limit."""

from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from pydantic import ValidationError

ROOT = Path(__file__).resolve().parents[3]


def test_preinstall_resource_budgets_cover_their_encoded_input() -> None:
    settings = gate_config.load(ROOT).package.preinstall_resource
    encoded = 4 * ((settings.max_binary_bytes + 2) // 3)
    assert settings.max_encoded_bytes >= encoded + (encoded + 75) // 76
    assert settings.max_control_bytes > settings.max_script_bytes > settings.max_encoded_bytes
    assert settings.timeout_seconds > settings.terminate_grace_seconds > 0


@pytest.mark.parametrize(
    "changes",
    [
        {"max_binary_bytes": 0},
        {"max_encoded_bytes": 1},
        {"max_script_bytes": 1},
        {"max_control_bytes": 1},
        {"timeout_seconds": 0},
        {"terminate_grace_seconds": 0},
        {"terminate_grace_seconds": 1000},
    ],
)
def test_inconsistent_or_unbounded_resource_configuration_is_refused(changes: dict) -> None:
    settings = gate_config.load(ROOT).package.preinstall_resource
    with pytest.raises(ValidationError):
        type(settings).model_validate(settings.model_dump() | changes)
