"""Citadel guard: policy files are published atomically.

Profile sources (profile.toml, enforcement.toml), corp config, user settings
and each session's active profile were written with `fs::write`, which
truncates the destination and then fills it. A reader between the two -- a
reload in capsem-process, a concurrent profile load in the service -- saw an
empty or half-written policy, and a crash in between left it that way
(google/capsem#202, owned by #229).

`capsem_foundation::unix::fs::atomic_write_private` writes a synced sibling
and renames it over the destination, so a reader sees the old complete file
or the new one. It is the one publication path for these modules.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]

RATIONALE = """\
Policy-owning code must publish files with
capsem_foundation::unix::fs::atomic_write_private, never fs::write or a
hand-rolled temp-file-and-rename. A truncating write lets a concurrent reload
read a partial policy and a crash leave one behind.
"""

# Production modules that write policy, settings or active-profile files.
POLICY_WRITERS = (
    Path("crates/capsem-core/src/net/policy_config/profile_contract.rs"),
    Path("crates/capsem-core/src/net/policy_config/corp_provision.rs"),
    Path("crates/capsem-core/src/net/policy_config/loader.rs"),
    Path("crates/capsem-service/src/main.rs"),
    Path("crates/capsem-service/src/profile_routes.rs"),
)
NON_ATOMIC_WRITE = re.compile(r"\bfs::write\s*\(|\bfs::rename\s*\(")
# The service writes a pid file and archives checkpoints; neither is policy.
ALLOWED = (
    re.compile(r"fs::write\(&path, pid\.to_string\(\)\)"),
    re.compile(r"fs::rename\(&checkpoint_path, &archived_path\)"),
    re.compile(r"fs::rename\(&complete_path, &archived_complete_path\)"),
)


def non_atomic_policy_writes(root: Path = PROJECT_ROOT) -> list[str]:
    found: list[str] = []
    for relative in POLICY_WRITERS:
        for number, line in enumerate((root / relative).read_text(encoding="utf-8").splitlines(), 1):
            code = line.split("//", 1)[0]
            if NON_ATOMIC_WRITE.search(code) and not any(allowed.search(code) for allowed in ALLOWED):
                found.append(f"{relative}:{number}: {code.strip()}")
    return found


def test_policy_publication_is_atomic() -> None:
    found = non_atomic_policy_writes()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def test_guard_detects_a_truncating_write(tmp_path: Path) -> None:
    for relative in POLICY_WRITERS:
        (tmp_path / relative).parent.mkdir(parents=True, exist_ok=True)
        (tmp_path / relative).write_text("fn ok() {}\n", encoding="utf-8")
    target = tmp_path / POLICY_WRITERS[0]
    target.write_text('fn save() { fs::write(&path, content).unwrap(); }\n', encoding="utf-8")
    assert non_atomic_policy_writes(tmp_path) == [
        f"{POLICY_WRITERS[0]}:1: fn save() {{ fs::write(&path, content).unwrap(); }}"
    ]
