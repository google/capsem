"""Citadel guard: a Cargo.lock change is chosen, not incidental.

The lock's identity is an input to the guest Rust builder image, and through
it to every guest agent, the initrd and the host binaries. A commit that
moves the lock as a side effect -- a new dependency added for one crate --
silently turns the next gate run from a cache hit into a ten-minute rebuild.
This guard fails on any lock that differs from the recorded one, so the
change is made once, on purpose, with its reason beside it.
"""

from __future__ import annotations

import hashlib
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
LOCK = PROJECT_ROOT / "Cargo.lock"
RECORD = Path(__file__).with_name("cargo_lock_identity.toml")

RATIONALE = """\
Cargo.lock changed without its record.

A lock change rebuilds the guest Rust builder image, every guest agent, the
initrd and every host binary on the next gate run. If this change is intended,
update tests/citadel/cargo_lock_identity.toml in the same commit: set `sha256`
to the value below and say in `reason` which dependency moved and why. If it
is not intended, restore the lock -- a stray `cargo update` or an added
dependency is exactly what this guard exists to catch.
"""


def lock_fingerprint(lock_bytes: bytes) -> str:
    return hashlib.sha256(lock_bytes).hexdigest()


def record() -> dict[str, str]:
    return tomllib.loads(RECORD.read_text(encoding="utf-8"))


def violations(lock_bytes: bytes, recorded: dict[str, str]) -> list[str]:
    actual = lock_fingerprint(lock_bytes)
    problems: list[str] = []
    if not recorded.get("reason", "").strip():
        problems.append("the record carries no reason for the current lock")
    if recorded.get("sha256") != actual:
        problems.append(
            f"recorded sha256 {recorded.get('sha256')!r} but Cargo.lock is {actual}"
        )
    return problems


def test_cargo_lock_changes_are_deliberate() -> None:
    problems = violations(LOCK.read_bytes(), record())
    assert not problems, RATIONALE + "\n" + "\n".join(problems)


def test_a_moved_lock_is_refused_until_its_record_moves_with_it() -> None:
    recorded = record()
    moved = LOCK.read_bytes() + b'\n[[package]]\nname = "stray"\nversion = "0.0.1"\n'
    problems = violations(moved, recorded)
    assert any("recorded sha256" in problem for problem in problems), problems
    caught_up = {**recorded, "sha256": lock_fingerprint(moved)}
    assert violations(moved, caught_up) == []


def test_a_record_without_a_reason_is_refused() -> None:
    recorded = {**record(), "reason": "  "}
    problems = violations(LOCK.read_bytes(), recorded)
    assert problems == ["the record carries no reason for the current lock"]
