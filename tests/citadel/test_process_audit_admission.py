"""Citadel guard: capsem-process never records audit rows by blocking a runtime worker.

The blocking audit path sends on a bounded std channel. On a Tokio worker it
parks the whole worker while the writer queue is full, so a guest flooding one
rail stalls unrelated tasks of its VM owner; its result was also ignored, so a
refused record went unnoticed (google/capsem#225, owned by #229). Async code
records through the async path and acts on refusal. The only blocking caller is
the guest audit stream, which runs on its own OS thread and should apply
backpressure to the guest that fills it.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROCESS_SRC = Path("crates/capsem-process/src")

RATIONALE = """\
Record audit rows from async capsem-process code with the async emitters
(emit_security_write, emit_*_security_write_and_rules) and refuse the action
when they return None. The blocking emitters park a Tokio worker on a full
writer queue. The one exception is the guest audit stream's own OS thread.
"""

BLOCKING = re.compile(r"\bemit_[a-z_]*_blocking\s*\(|\bwrite_blocking(?:_checked)?\s*\(")
#: (file, the function allowed to call a blocking emitter, and why).
ALLOWED = {(PROCESS_SRC / "vsock" / "audit.rs", "handle_audit_frame")}
FUNCTION = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?(?:async\s+)?fn\s+(\w+)")


def blocking_audit_calls(root: Path = PROJECT_ROOT) -> list[str]:
    found: list[str] = []
    for path in sorted((root / PROCESS_SRC).rglob("*.rs")):
        relative = path.relative_to(root)
        if "tests" in relative.parts or path.name == "tests.rs":
            continue
        function = None
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if match := FUNCTION.match(line):
                function = match.group(1)
            code = line.split("//", 1)[0]
            if BLOCKING.search(code) and (relative, function) not in ALLOWED:
                found.append(f"{relative}:{number} in {function}: {code.strip()}")
    return found


def test_process_audit_rows_are_admitted_without_blocking_a_runtime_worker() -> None:
    found = blocking_audit_calls()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def test_guard_detects_a_blocking_emit_in_async_code(tmp_path: Path) -> None:
    (tmp_path / PROCESS_SRC / "vsock").mkdir(parents=True)
    (tmp_path / PROCESS_SRC / "vsock" / "audit.rs").write_text(
        "fn handle_audit_frame() {\n    emit_process_audit_security_write_and_rules_blocking(db);\n}\n",
        encoding="utf-8",
    )
    (tmp_path / PROCESS_SRC / "ipc.rs").write_text(
        "async fn serve() {\n    emit_file_security_write_and_rules_blocking(db, rules, event);\n}\n",
        encoding="utf-8",
    )
    assert [line.split(": ", 1)[0] for line in blocking_audit_calls(tmp_path)] == [
        f"{PROCESS_SRC / 'ipc.rs'}:2 in serve",
    ]
