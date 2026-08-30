"""Contracts for Python readers of Capsem's structured log streams."""

import pytest
from log_streams import assert_service_log_evidence


@pytest.mark.parametrize("target", ["service", "capsem_service"])
def test_service_log_evidence_accepts_current_owned_targets(target: str) -> None:
    assert_service_log_evidence(f'{{"target":"{target}"}}\n')


@pytest.mark.parametrize(
    "text",
    ["", "not json\n", '{"target":"capsem_gateway"}\n'],
)
def test_service_log_evidence_rejects_missing_owned_records(text: str) -> None:
    with pytest.raises(AssertionError):
        assert_service_log_evidence(text)
