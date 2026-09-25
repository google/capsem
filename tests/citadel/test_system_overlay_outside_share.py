"""The guest's system overlay image stays out of the VirtioFS share.

The image is attached to the guest as a writable disk by path. While it lived
at `guest/system/rootfs.img`, inside the read-write share, a root guest could
mount the share and replace it with a symlink to any host file; the next boot
attached that file as the guest's disk, and host routes read through the link.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

RATIONALE = """\
The system overlay image lives in the session's host-only `system/`
directory. Every production reference goes through capsem_core::session
(`system_overlay_image_path`, `open_system_overlay`, `system_overlay_metadata`,
`adopt_system_overlay`), which alone knows the layout and migrates the legacy
in-share image without following a link. Spelling the path elsewhere, or
reaching it through the guest share, reopens a guest-to-host disk escape.
"""

OWNER = "crates/capsem-core/src/session/overlay.rs"
# Any production spelling of the overlay's location, in or out of the share.
LAYOUT = re.compile(
    r"system/rootfs\.img"
    r"|join\(\s*\"system\"\s*\)"
    r"|guest_share_dir\([^)]*\)\s*\.join\(\s*\"system"
    r"|\"guest\"\s*\)\s*\.join\(\s*\"system\""
)


def _is_test(path: Path) -> bool:
    return "tests" in path.parts or path.name == "tests.rs" or "examples" in path.parts


def _hits(root: Path = ROOT) -> list[str]:
    hits = []
    for path in sorted((root / "crates").rglob("*.rs")):
        rel = path.relative_to(root)
        if _is_test(rel) or rel.as_posix() == OWNER:
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            code = line.split("//", 1)[0]
            if LAYOUT.search(code):
                hits.append(f"{rel.as_posix()}:{number}: {line.strip()}")
    return hits


def test_only_the_overlay_module_spells_the_image_location() -> None:
    hits = _hits()
    assert not hits, "system overlay path spelled outside its owner:\n" + "\n".join(hits) + "\n\n" + RATIONALE


def test_the_owner_still_defines_the_host_only_layout() -> None:
    owner = (ROOT / OWNER).read_text(encoding="utf-8")
    for needle in ("pub fn adopt_system_overlay", "pub fn open_system_overlay", "remove_symlink", "rename_to"):
        assert needle in owner, f"{OWNER} lost {needle}\n\n{RATIONALE}"


def test_the_guard_catches_every_spelling(tmp_path: Path) -> None:
    source = tmp_path / "crates" / "capsem-x" / "src"
    source.mkdir(parents=True)
    evasions = [
        'let p = dir.join("guest/system/rootfs.img");',
        'let p = guest_share_dir(&d).join("system/rootfs.img");',
        'let p = capsem_core::guest_share_dir(&d).join("system").join("rootfs.img");',
        'let p = d.join("guest").join("system").join("rootfs.img");',
        'let p = session_dir.join( "system" ).join(IMAGE);',
    ]
    for index, line in enumerate(evasions):
        (source / f"e{index}.rs").write_text(f"fn f() {{ {line} }}\n")
    (source / "fine.rs").write_text('// system/rootfs.img in a comment\nfn f() { overlay::open_system_overlay(&d); }\n')

    hits = _hits(tmp_path)

    assert len(hits) == len(evasions), hits
    assert not any("fine.rs" in hit for hit in hits)
