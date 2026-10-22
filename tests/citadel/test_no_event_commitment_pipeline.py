"""The retired content-commitment pipeline must not return to ledger admission."""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVENT_COMMITMENT_RATIONALE = """\
Ledger admission uses authenticated, role-bound descriptors and ordered requests.
Per-event JSON hashes and reservation/anchor side channels did not independently
verify stored event contents. They expanded binary output, repeated CPU work,
copied bodies and added round trips and threads to every producer. Keep this
machinery out; see AGENTS.md 'Code Style' and skills/dev-rust-patterns.
"""
RETIRED = re.compile(r"\b(?:ledger_commitment|commitment_event_hash|CommitmentState|verify_host_chain|chain_hash|sql_fingerprint)\b")


def retired_references(source: str) -> bool:
    return RETIRED.search(source) is not None


def test_event_commitments_are_retired() -> None:
    found = [
        str(path.relative_to(ROOT))
        for path in (ROOT / "crates").rglob("*.rs")
        if retired_references(path.read_text())
    ]
    assert not found, EVENT_COMMITMENT_RATIONALE + "\n" + "\n".join(sorted(found))


def test_guard_rejects_reintroduced_hash_and_side_channel() -> None:
    assert retired_references("let hash = commitment_event_hash(&event)?;")
    assert retired_references("use crate::ledger_commitment::CommitmentAuthority;")
    assert retired_references("let state = CommitmentState::new(sender, receiver, grant);")
    assert retired_references("let digest = chain_hash(&previous, &event);")
    assert not retired_references("let digest = verify_downloaded_asset(&manifest)?;")
