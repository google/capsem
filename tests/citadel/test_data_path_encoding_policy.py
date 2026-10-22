"""JSON and content hashing need an exact, documented contract, not convenience."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
POLICY = Path(__file__).with_name("data_path_reasons.toml")
DATA_PATH_RATIONALE = """\
See AGENTS.md 'Code Style' and skills/dev-rust-patterns/references/data-path-performance.md.
Internal data paths use typed binary encoding, ownership transfer and one bounded
final frame. JSON and content hashes require a concrete external, durable-format,
storage-integrity or compatibility contract. Document the reason in the exact
source inventory; new uses and growth require review, never a blanket exemption.
Do not bless a new use by mechanically increasing the count.
"""


def code(source: str) -> str:
    source = re.sub(r"/\*.*?\*/", " ", source, flags=re.S)
    source = re.sub(r"//[^\n]*", "", source)
    for namespace in ("serde_json", "blake3", "xxh3", "Sha256"):
        for alias in re.findall(rf"\b{namespace}\s+as\s+(\w+)", source):
            source = re.sub(rf"\b{alias}\s*::", namespace + "::", source)
    for function, alias in re.findall(r"\b(to_\w+|from_\w+)\s+as\s+(\w+)", source):
        if "serde_json" in source:
            source = re.sub(rf"\b{alias}\s*\(", "serde_json::" + function + "(", source)
    return source


def uses(source: str) -> dict[str, int]:
    source = code(source)
    return {
        "json": len(re.findall(r"\bserde_json\s*::|\bjson\s*!", source)),
        "hash": len(re.findall(
            r"\b(?:blake3|xxh3)\s*::|\bSha256\s*::\s*(?:new|digest|default)\b|\b(?:blake3_(?:bytes_ref|ref)|content_digest|fingerprint_fields|metadata_fingerprint)\s*\(", source,
        )),
    }


def production_sources(root: Path) -> list[Path]:
    return sorted(
        path
        for folder in (root / "crates", root / "sdk/rust")
        for path in folder.rglob("*.rs")
        if not any("test" in part or part in {"benches", "examples"} for part in path.relative_to(folder).parts)
    )


def violations(source: str, approved: dict, reasons: dict) -> list[str]:
    found = []
    for kind, count in uses(source).items():
        entry = approved.get(kind)
        if count == 0:
            if entry:
                found.append(f"{kind}: stale allowance; remove it")
            continue
        if not entry or entry.get("count") != count:
            found.append(f"{kind}: {count} uses require exact reviewed allowance")
        if entry and entry.get("reason") not in reasons:
            found.append(f"{kind}: missing documented contract")
    return found


def test_json_and_hashing_have_exact_documented_reasons() -> None:
    policy = tomllib.loads(POLICY.read_text())
    reasons = policy["reasons"]
    approved = policy.get("sources", {})
    found = []
    observed = set()
    for path in production_sources(ROOT):
        relative = path.relative_to(ROOT).as_posix()
        observed.add(relative)
        found.extend(f"{relative}: {message}" for message in violations(path.read_text(), approved.get(relative, {}), reasons))
    found.extend(f"{path}: stale source allowance" for path in approved.keys() - observed)
    assert not found, DATA_PATH_RATIONALE + "\n" + "\n".join(found)
    assert all(len(reason) >= 50 for reason in reasons.values()), "Document the actual contract, not a label"


@pytest.mark.parametrize("source,kind", [
    ("let data = serde_json::to_vec(&event)?;", "json"),
    ("use serde_json as json_codec; let data = json_codec::to_vec(&event)?;", "json"),
    ("use serde_json::{to_vec as encode}; let data = encode(&event)?;", "json"),
    ("let digest = capsem_crypto::blake3::hash(&bytes);", "hash"),
    ("use capsem_crypto::blake3 as digest; let hash = digest::hash(&bytes);", "hash"),
    ("let digest = Sha256::new();", "hash"),
    ("let digest = Sha256::digest(&bytes);", "hash"),
    ("let digest = capsem_crypto::xxh3::hash(&bytes);", "hash"),
    ("use capsem_crypto::xxh3 as quick; let digest = quick::hash(&bytes);", "hash"),
    ("let digest = content_digest(&bytes);", "hash"),
    ("let digest = fingerprint_fields(&fields);", "hash"),
    ("let digest = metadata_fingerprint(&record, domain);", "hash"),
])
def test_new_or_aliased_uses_require_a_reason(source: str, kind: str) -> None:
    assert uses(source)[kind] > 0
    assert violations(source, {}, {})


def test_growth_and_stale_allowances_are_rejected() -> None:
    source = "serde_json::to_vec(&event)?;"
    reasons = {"external": "The external HTTP JSON response contract requires this encoding."}
    approved = {"json": {"count": 1, "reason": "external"}}
    assert not violations(source, approved, reasons)
    assert violations(source * 2, approved, reasons)
    assert violations("", approved, reasons)
    assert violations(source, approved, {})
    assert uses("// serde_json::to_vec(&event)?;\n/* blake3::hash(bytes) */") == {"json": 0, "hash": 0}


def test_terminal_and_admission_do_not_hash_or_use_json() -> None:
    paths = [
        *[p for p in (ROOT / "crates/capsem-core/src/terminal").rglob("*.rs") if "tests" not in p.parts and p.name != "tests.rs"],
        ROOT / "crates/capsem-logger/src/ledger_protocol.rs",
        ROOT / "crates/capsem-logger/src/writer/remote.rs",
    ]
    assert all(not any(uses(path.read_text()).values()) for path in paths), DATA_PATH_RATIONALE


def query_metrics_reparse(source: str) -> bool:
    return bool(re.search(r"\bserde_json\s*::\s*from_\w+\s*(?:::<[^;]+?>)?\s*\(", code(source)))


def test_query_metrics_carry_counts_without_reparsing_the_encoded_result() -> None:
    paths = ["crates/capsem-logger/src/db.rs", "crates/capsem-logger/src/db/reader_worker.rs"]
    assert not any(query_metrics_reparse((ROOT / path).read_text()) for path in paths), (
        "The SQLite reader already knows the row count. Carry it with the encoded "
        "result instead of rebuilding a JSON DOM in execute/handle telemetry. " + DATA_PATH_RATIONALE
    )


@pytest.mark.parametrize("source", [
    "serde_json::from_str::<serde_json::Value>(raw)",
    "serde_json::from_slice(bytes)",
    "use serde_json as codec; codec::from_str(raw)",
    "use serde_json::{from_str as decode}; decode(raw)",
])
def test_query_reparse_guard_catches_direct_and_aliased_calls(source: str) -> None:
    assert query_metrics_reparse(source)
