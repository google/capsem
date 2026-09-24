"""The host never reaches into the guest's VirtioFS share by path.

The share (`<session>/guest/`) is read-write for the guest, so the guest can
replace any entry in it -- the workspace itself, a file the host is about to
read -- with a symlink to a host path. Opening a path follows that link: the
files API once served and wrote a linked host directory as the workspace, the
file monitor walked it into the ledger and brokered its `.env`, and the doctor
bundle copied whatever a planted link named.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

RATIONALE = """\
Reach the share through descriptors from the host-owned session directory:
capsem_core::session::open_workspace for the workspace, and
ContainedDir::open_root(session_dir) then descend/open_file for anything else.
The only path use allowed is handing the share to the hypervisor as the
VirtioFS root (capsem-process's session layout).
"""

# The runtime crates that touch a live session. Image building in
# capsem-admin has its own unrelated `guest` directory.
SCANNED = ("crates/capsem-core/src", "crates/capsem-process/src", "crates/capsem-service/src",
           "crates/capsem/src", "crates/capsem-gateway/src")
ALLOWED = {
    # The definition.
    "crates/capsem-core/src/lib.rs": 1,
    # The VirtioFS share root handed to the hypervisor.
    "crates/capsem-process/src/main.rs": 1,
}
BY_PATH = re.compile(r"guest_share_dir\(|join\(\s*\"guest\"\s*\)|\"guest/")


def _is_test(path: Path) -> bool:
    return "tests" in path.parts or path.name == "tests.rs"


def _hits(root: Path = ROOT) -> dict[str, list[str]]:
    hits: dict[str, list[str]] = {}
    for entry in SCANNED:
        for path in sorted((root / entry).rglob("*.rs")):
            rel = path.relative_to(root)
            if _is_test(rel):
                continue
            for number, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
                if BY_PATH.search(line.split("//", 1)[0]):
                    hits.setdefault(rel.as_posix(), []).append(f"{rel.as_posix()}:{number}: {line.strip()}")
    return hits


def _violations(hits: dict[str, list[str]]) -> list[str]:
    return [line for name, lines in hits.items() if len(lines) > ALLOWED.get(name, 0) for line in lines]


def test_the_guest_share_is_reached_only_by_descriptor() -> None:
    violations = _violations(_hits())
    assert not violations, "guest share reached by path:\n" + "\n".join(violations) + "\n\n" + RATIONALE


def test_the_allowed_uses_still_exist() -> None:
    hits = _hits()
    stale = [name for name, count in ALLOWED.items() if len(hits.get(name, [])) != count]
    assert not stale, f"allowlist no longer matches the tree, tighten it: {stale}\n\n{RATIONALE}"


def test_the_guard_catches_every_spelling(tmp_path: Path) -> None:
    source = tmp_path / "crates" / "capsem-service" / "src"
    source.mkdir(parents=True)
    evasions = [
        'let w = capsem_core::guest_share_dir(&d).join("workspace");',
        'let w = session_dir.join("guest").join("workspace");',
        'let w = session_dir.join( "guest" );',
        'let b = session_dir.join("guest/doctor-bundle.tar");',
    ]
    for index, line in enumerate(evasions):
        (source / f"e{index}.rs").write_text(f"fn f() {{ {line} }}\n")
    (source / "fine.rs").write_text('// guest_share_dir( in a comment\nfn f() { open_workspace(&d); }\n')

    violations = _violations(_hits(tmp_path))

    assert len(violations) == len(evasions), violations
