"""Ironbank credential broker ledger contract tests."""

from __future__ import annotations

import pytest

from tests.ironbank.test_http_protocol_ledger import (
    test_brokered_http_rewrite_pays_full_ledger_debt_blackbox as _broker_rewrite_proof,
)


pytestmark = pytest.mark.integration


def test_credential_broker_capture_injects_and_reports_full_ledger_blackbox() -> None:
    """Dedicated S01-005 entry point for the broker rewrite ledger proof."""
    _broker_rewrite_proof()
