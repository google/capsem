#!/usr/bin/env python3
"""Causal installed-update evidence shared by Linux and macOS release gates."""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import cast

VERDICT_SCHEMA = "capsem.release_transition_verdict.v1"
UPDATE_AUDIT_SCHEMA = "capsem.update_audit.v1"
SHA256 = re.compile(r"^[0-9a-f]{64}$")


class TransitionEvidenceError(RuntimeError):
    """The update audit did not prove the requested exact transition."""


def load_update_audit(path: Path, *, after_line: int = 0) -> list[dict[str, object]]:
    if after_line < 0:
        raise TransitionEvidenceError("update audit line marker must not be negative")
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except FileNotFoundError:
        return []
    except OSError as error:
        raise TransitionEvidenceError(f"cannot read update audit {path}: {error}") from error
    rows: list[dict[str, object]] = []
    for line_number, line in enumerate(lines[after_line:], start=after_line + 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as error:
            raise TransitionEvidenceError(
                f"update audit {path}:{line_number} is invalid JSON: {error}"
            ) from error
        if not isinstance(row, dict):
            raise TransitionEvidenceError(
                f"update audit {path}:{line_number} must contain an object"
            )
        rows.append(cast(dict[str, object], row))
    return rows


def _state_digest(row: Mapping[str, object], field: str) -> str | None:
    state = row.get(field)
    if not isinstance(state, Mapping):
        return None
    digest = cast(Mapping[str, object], state).get("manifest_sha256")
    return digest if isinstance(digest, str) else None


def _matches(row: Mapping[str, object], source: str, digest: str) -> bool:
    return (
        row.get("schema") == UPDATE_AUDIT_SCHEMA
        and row.get("source") == source
        and row.get("candidate_manifest_sha256") == digest
    )


def build_transition_verdict(
    rows: Sequence[Mapping[str, object]],
    *,
    kind: str, result: str, source: str,
    candidate_manifest_sha256: str,
    previous_manifest_sha256: str | None = None,
) -> dict[str, object]:
    """Bind a verdict to the exact bytes fetched and handled by the product."""
    if not source:
        raise TransitionEvidenceError("transition candidate source must not be empty")
    if SHA256.fullmatch(candidate_manifest_sha256) is None:
        raise TransitionEvidenceError("candidate manifest SHA-256 must be a lowercase digest")
    if result not in {"activated", "rejected"}:
        raise TransitionEvidenceError("transition verdict must be activated or rejected")
    matching = [
        (index, row)
        for index, row in enumerate(rows)
        if _matches(row, source, candidate_manifest_sha256)
    ]
    fetched = next(
        (
            (index, row)
            for index, row in matching
            if row.get("event") in {"release_candidate_fetched", "asset_update_start"}
        ),
        None,
    )
    if fetched is None:
        raise TransitionEvidenceError(
            "update audit does not prove that the product fetched the exact candidate"
        )
    terminal_events = (
        {"release_candidate_activated", "asset_update_complete"}
        if result == "activated"
        else {"release_candidate_rejected"}
    )
    terminal = next(
        (
            row
            for index, row in matching
            if index > fetched[0] and row.get("event") in terminal_events
        ),
        None,
    )
    if terminal is None:
        raise TransitionEvidenceError(
            f"update audit has no exact-candidate {result} event after its fetch"
        )

    preserved = False
    if result == "activated":
        if _state_digest(terminal, "current") != candidate_manifest_sha256:
            raise TransitionEvidenceError(
                "candidate activation event does not identify the installed candidate manifest"
            )
    else:
        if previous_manifest_sha256 is None or SHA256.fullmatch(previous_manifest_sha256) is None:
            raise TransitionEvidenceError(
                "rejection verdict requires the previous installed manifest SHA-256"
            )
        if (
            _state_digest(terminal, "previous") != previous_manifest_sha256
            or _state_digest(terminal, "current") != previous_manifest_sha256
        ):
            raise TransitionEvidenceError(
                "candidate rejection did not preserve the exact previous manifest"
            )
        error = terminal.get("error")
        if not isinstance(error, str) or not error:
            raise TransitionEvidenceError("candidate rejection event has no causal error")
        required = {
            "tampered_artifact": (
                "mismatch",
                "failed size or digest verification",
            ),
            "incompatible_profile": ("requires Capsem 9999.0.0 or newer",),
        }.get(kind)
        if required is not None and not any(
            cause.lower() in error.lower() for cause in required
        ):
            raise TransitionEvidenceError(
                f"{kind} rejection event does not name its exact rejection cause"
            )
        preserved = True

    verdict: dict[str, object] = {
        "schema": VERDICT_SCHEMA,
        "kind": kind,
        "result": result,
        "source": source,
        "candidate_manifest_sha256": candidate_manifest_sha256,
        "fetched": True,
        "installed_manifest_sha256": _state_digest(terminal, "current"),
        "preserved_previous": preserved,
        "fetch_event": fetched[1].get("event"),
        "terminal_event": terminal.get("event"),
    }
    if previous_manifest_sha256 is not None:
        verdict["previous_manifest_sha256"] = previous_manifest_sha256
    return verdict


def validate_transition_verdict(
    evidence: Mapping[str, object],
    *,
    kind: str,
    result: str,
    source: str,
    candidate_manifest_sha256: str,
    previous_manifest_sha256: str | None = None,
) -> Mapping[str, object]:
    expected = {
        "schema": VERDICT_SCHEMA,
        "kind": kind,
        "result": result,
        "source": source,
        "candidate_manifest_sha256": candidate_manifest_sha256,
        "fetched": True,
        "preserved_previous": result == "rejected",
    }
    for field, value in expected.items():
        if evidence.get(field) != value:
            raise TransitionEvidenceError(
                f"transition verdict {field} is {evidence.get(field)!r}, expected {value!r}"
            )
    installed = evidence.get("installed_manifest_sha256")
    expected_installed = (
        candidate_manifest_sha256 if result == "activated" else previous_manifest_sha256
    )
    if expected_installed is None or installed != expected_installed:
        raise TransitionEvidenceError(
            "transition verdict does not identify the exact resulting installed manifest"
        )
    if result == "rejected" and evidence.get("previous_manifest_sha256") != expected_installed:
        raise TransitionEvidenceError(
            "transition rejection verdict does not identify the preserved previous manifest"
        )
    return evidence


def validate_tamper_verdicts(
    fresh: Mapping[str, object],
    rejection: Mapping[str, object],
    *,
    source: str,
    installed_manifest_sha256: str,
) -> None:
    """Require causal fresh activation and distinct-byte tamper rejection."""
    validate_transition_verdict(
        fresh,
        kind="fresh_install",
        result="activated",
        source=source,
        candidate_manifest_sha256=installed_manifest_sha256,
    )
    rejected = rejection.get("candidate_manifest_sha256")
    if not isinstance(rejected, str) or rejected == installed_manifest_sha256:
        raise TransitionEvidenceError("tamper verdict does not identify distinct candidate bytes")
    validate_transition_verdict(
        rejection,
        kind="tampered_artifact",
        result="rejected",
        source=source,
        candidate_manifest_sha256=rejected,
        previous_manifest_sha256=installed_manifest_sha256,
    )


def wait_for_transition_verdict(
    audit_path: Path,
    *,
    after_line: int,
    timeout_seconds: float,
    kind: str,
    result: str,
    source: str,
    candidate_manifest_sha256: str,
    previous_manifest_sha256: str | None = None,
) -> dict[str, object]:
    deadline = time.monotonic() + timeout_seconds
    last_error: TransitionEvidenceError | None = None
    while True:
        try:
            return build_transition_verdict(
                load_update_audit(audit_path, after_line=after_line),
                kind=kind,
                result=result,
                source=source,
                candidate_manifest_sha256=candidate_manifest_sha256,
                previous_manifest_sha256=previous_manifest_sha256,
            )
        except TransitionEvidenceError as error:
            last_error = error
        if time.monotonic() >= deadline:
            raise TransitionEvidenceError(
                f"timed out waiting for exact transition evidence: {last_error}"
            ) from last_error
        time.sleep(1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--audit-log", dest="audit_path", required=True, type=Path)
    parser.add_argument("--after-line", required=True, type=int)
    parser.add_argument("--kind", required=True)
    parser.add_argument("--result", required=True, choices=("activated", "rejected"))
    parser.add_argument("--source", required=True)
    parser.add_argument("--candidate-manifest-sha256", required=True)
    parser.add_argument("--previous-manifest-sha256")
    parser.add_argument("--timeout-seconds", type=float, default=180)
    parser.add_argument("--evidence-out", required=True, type=Path)
    args = parser.parse_args()
    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be positive")
    output = args.evidence_out
    verdict = wait_for_transition_verdict(
        args.audit_path,
        after_line=args.after_line,
        timeout_seconds=args.timeout_seconds,
        kind=args.kind,
        result=args.result,
        source=args.source,
        candidate_manifest_sha256=args.candidate_manifest_sha256,
        previous_manifest_sha256=args.previous_manifest_sha256,
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    pending = output.with_name(f".{output.name}.tmp")
    pending.write_text(json.dumps(verdict, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    pending.replace(output)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except TransitionEvidenceError as error:
        print(f"release transition evidence failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
