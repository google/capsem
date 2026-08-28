"""Hardlinking from Rust goes through one audited place.

Python's primitives are proxied (`capsem_builder.gate.observation.Instrument`), so
every in-process effect is observable by construction rather than by
remembering to log it. Rust cannot be monkeypatched; the equivalent is a
chokepoint plus this test.

**Why this is scoped to `hard_link` and not to mutation generally.** The first
version forbade every `fs::` mutation outside an audited module and found 259
call sites -- most of them runtime service code removing its own sockets and
session directories, which has nothing to do with what a release qualifies.
Routing all of that through an audit layer would have been a large refactor
bought with no safety. Narrowing afterwards to whatever made the test pass
would have been worse: a guard shaped around its own result.

So the invariant is the one that actually failed. A hardlink is the only
operation that makes two paths *the same file*, and there are exactly two in
the workspace. `capsem-admin` used one to stage profile payloads and put 48
checked-in `config/` files inside published release output -- one inode, so a
chmod on the artifact rewrites tracked source and no content digest notices.
Auditing every hardlink is cheap precisely because hardlinks are rare, and it
closes the class completely.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]

#: The module that owns linking, and may call the primitive.
AUDITED = "crates/capsem-core/src/auditfs.rs"

#: Named, not silent. The guest's own filesystem inside its own share: the
#: guest issues the link, the host is implementing a syscall for it, and no
#: release artifact is involved. An exemption nobody can see is how a
#: chokepoint stops being one.
EXEMPT = {"crates/capsem-core/src/hypervisor/kvm/virtio_fs/ops_dir.rs"}

LINK = re.compile(r"\bfs::hard_link\s*\(")


def _rust_sources() -> list[Path]:
    return [
        path
        for path in (PROJECT_ROOT / "crates").rglob("*.rs")
        if "target" not in path.parts and not path.name.endswith("tests.rs")
    ]


def test_hardlinking_goes_through_the_audited_module() -> None:
    """Two paths becoming one file is worth one place in the codebase."""
    offenders: list[str] = []
    for path in _rust_sources():
        relative = path.relative_to(PROJECT_ROOT).as_posix()
        if relative == AUDITED or relative in EXEMPT:
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if LINK.search(line):
                offenders.append(f"{relative}:{number}: {line.strip()}")

    assert not offenders, (
        f"{len(offenders)} unaudited hardlink(s). A hardlink makes two paths the "
        "same file, which is how checked-in source ended up inside published "
        "release output. Route them through capsem_core::auditfs:\n  "
        + "\n  ".join(offenders)
    )


def test_the_exemptions_still_exist() -> None:
    """An allowlist that outlives the code it names silently re-opens the hole
    it was granted for."""
    for relative in EXEMPT:
        assert (PROJECT_ROOT / relative).is_file(), f"{relative} is exempt but gone"
