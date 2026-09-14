"""Citadel guard: the typed IPC transport is built in one place.

`tokio_unix_ipc` closes a channel's descriptor before deregistering it from
the runtime, and on a busy runtime the number is reused in between, so the
next socket on that number never reports readiness. One private admission in
about two thousand sat in `recv` until the service gave up (gate
20260912-190634). `capsem_foundation::ipc_channel` takes the descriptor back
before the transport can close it; a channel built anywhere else is that
defect reintroduced on a path the stress tests do not cover.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CRATES = PROJECT_ROOT / "crates"

RATIONALE = """\
The typed IPC transport is built only by capsem_foundation::ipc_channel.

tokio_unix_ipc's Sender and Receiver close their descriptor in Drop before
the AsyncFd inside deregisters it; between the two, another task on the same
runtime can reuse the number, and the stale deregistration removes the new
socket's readiness interest for good. The foundation channel releases the
descriptor in the right order. Build channels with
capsem_foundation::ipc_channel::channel_from_std and name its Sender and
Receiver; do not depend on tokio-unix-ipc directly.
"""

OWNER = Path("crates/capsem-foundation/src/ipc_channel.rs")
OWNER_MANIFEST = Path("crates/capsem-foundation/Cargo.toml")
TRANSPORT_REFERENCE = re.compile(r"\btokio_unix_ipc\b")
TRANSPORT_DEPENDENCY = re.compile(r"^\s*tokio-unix-ipc\b", re.MULTILINE)


def _code_lines(path: Path) -> list[str]:
    lines: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        code = line.split("//", 1)[0].strip()
        if code:
            lines.append(code)
    return lines


def transport_references(root: Path = PROJECT_ROOT) -> list[str]:
    found: list[str] = []
    for path in sorted((root / "crates").rglob("*.rs")):
        relative = path.relative_to(root)
        if relative == OWNER or relative.parent == root_owner_dir():
            continue
        for code in _code_lines(path):
            if TRANSPORT_REFERENCE.search(code):
                found.append(f"{relative}: {code}")
    return found


def root_owner_dir() -> Path:
    return OWNER.with_suffix("")


def transport_dependencies(root: Path = PROJECT_ROOT) -> list[str]:
    found: list[str] = []
    for manifest in sorted((root / "crates").glob("*/Cargo.toml")):
        relative = manifest.relative_to(root)
        if relative == OWNER_MANIFEST:
            continue
        if TRANSPORT_DEPENDENCY.search(manifest.read_text(encoding="utf-8")):
            found.append(str(relative))
    return found


def test_only_the_foundation_channel_touches_the_transport() -> None:
    found = transport_references()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def test_only_the_foundation_crate_depends_on_the_transport() -> None:
    found = transport_dependencies()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def test_a_direct_channel_is_reported(tmp_path: Path) -> None:
    crate = tmp_path / "crates" / "capsem-elsewhere"
    (crate / "src").mkdir(parents=True)
    (crate / "src" / "main.rs").write_text(
        "// tokio_unix_ipc in prose is fine\n"
        "let (tx, rx) = tokio_unix_ipc::channel_from_std(stream)?;\n",
        encoding="utf-8",
    )
    (crate / "Cargo.toml").write_text('[dependencies]\ntokio-unix-ipc = "0.4"\n', encoding="utf-8")
    assert transport_references(tmp_path) == [
        "crates/capsem-elsewhere/src/main.rs: let (tx, rx) = tokio_unix_ipc::channel_from_std(stream)?;"
    ]
    assert transport_dependencies(tmp_path) == ["crates/capsem-elsewhere/Cargo.toml"]
