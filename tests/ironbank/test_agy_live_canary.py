"""Antigravity live-canary gate wiring.

AGY OAuth remains a manual live proof because the browser OAuth dance requires
Elie. This file keeps that contract executable without pretending manual OAuth
is a hermetic release gate.
"""

from __future__ import annotations

from tests.live_provider.test_live_provider_canaries import TRACKED_MANUAL_LIVE_CANARIES


def test_agy_live_oauth_canary_is_tracked_as_manual() -> None:
    assert TRACKED_MANUAL_LIVE_CANARIES == {
        "agy": "AGY OAuth live proof is tracked by S02-016 because it needs an interactive OAuth dance, not an env-key canary.",
    }
