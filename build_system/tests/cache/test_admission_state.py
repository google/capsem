"""Admission attempts and successful resets share the cache mutation journal."""

from pathlib import Path

from capsem_builder.cache.models import AdmissionEvent, AdmissionEventKind
from capsem_builder.cache.operations import last_admission_event, record_admission_event


def event(kind: AdmissionEventKind, reason: str = "") -> AdmissionEvent:
    return AdmissionEvent(
        kind=kind,
        timestamp_ns=1,
        source_identity="a" * 40,
        reason=reason,
    )


def test_latest_admission_event_roundtrips_strictly(tmp_path: Path) -> None:
    root = tmp_path / "cache"
    state = Path("state/test-admission.jsonl")
    forced = event(AdmissionEventKind.FORCED_ATTEMPT, "needed")
    success = event(AdmissionEventKind.COMPLETE_SUCCESS)

    record_admission_event(root, state, forced)
    assert last_admission_event(root, state) == forced
    record_admission_event(root, state, success)
    assert last_admission_event(root, state) == success


def test_missing_admission_state_has_no_prior_event(tmp_path: Path) -> None:
    assert last_admission_event(tmp_path / "cache", Path("state/events.jsonl")) is None
