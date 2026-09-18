"""Citadel guard: the body archive's on-disk format is written down once.

`capsem-archive/src/format.rs` is the whole description of a `.bodies` file:
the magic `CAPSEMBL`, the 16-byte file header, the per-block magic `BLK1`, and
the 44-byte block header that carries `raw_len`, `comp_len` and the blake3 of
the raw bytes.

A second copy of any of those four facts is not a duplicated constant, it is a
second specification. They drift in one direction only -- the writer is
changed, the copy is not -- and the file that results is one both halves think
they can read. There is no version negotiation to catch it either: a block
header is 44 bytes because `format.rs` says so, and a reader that believes 40
reads a hash out of the middle of someone's request body and reports
corruption in the wrong place.

The one allowed second copy is `tests/helpers/body_archive.py`, and it is
allowed because of what it is for: a black-box proof that parses the file
without using the crate that wrote it. A proof that imported the definition it
is checking would pass by construction. It is the one copy that must track
`format.rs` by hand, so it is listed here by name and with its reason rather
than matched by a pattern that would also let the next one through.

See skills/dev-session-debug and CLAUDE.md 'Logger DB Boundary'.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
FORMAT = Path("crates/capsem-archive/src/format.rs")
# Version 2 of the format, beside version 1 while the logger moves over; it
# replaces format.rs, and this entry goes, once nothing writes version 1.
FORMAT_V2 = Path("crates/capsem-archive/src/v2/format.rs")

# The four facts, and what *defining* one looks like. Using them is the point
# of their being public -- `use capsem_archive::BLOCK_HEADER_BYTES` is how a
# caller stays correct. What this guard is about is a second file that decides
# for itself what the number or the magic is.
FORMAT_FACTS: tuple[tuple[str, re.Pattern[str]], ...] = (
    # A magic is its own definition: a literal spelling it out anywhere else is
    # a parser that has stopped asking capsem-archive.
    ("CAPSEMBL", re.compile(r"""['"]CAPSEMBL['"]""")),
    ("BLK1", re.compile(r"""['"]BLK1['"]""")),
    # A binding, not a mention. `const X: usize = 16` and `X = 16` are second
    # definitions; a prefix is not a disguise, so ARCHIVE_FILE_HEADER_BYTES
    # counts too.
    ("FILE_HEADER_BYTES", re.compile(r"\b\w*FILE_HEADER_BYTES\b\s*(?::\s*\w+\s*)?=")),
    ("BLOCK_HEADER_BYTES", re.compile(r"\b\w*BLOCK_HEADER_BYTES\b\s*(?::\s*\w+\s*)?=")),
)

# path -> why this copy exists. Anything not named here is a violation.
ALLOWLIST: dict[Path, str] = {
    Path("tests/helpers/body_archive.py"): (
        "black-box proof of the format; must track format.rs"
    ),
}

FORMAT_IS_ONE_PLACE_RATIONALE = """\
The body archive's format is defined in crates/capsem-archive/src/format.rs.

CAPSEMBL, BLK1, FILE_HEADER_BYTES and BLOCK_HEADER_BYTES describe the bytes on
disk. A second copy is a second specification, and the two drift the moment
one side is changed: the writer emits a 44-byte block header, the stale reader
believes 40, and it reads a blake3 hash out of the middle of a request body
and calls the archive corrupt.

Import them from capsem-archive. The one exception is
tests/helpers/body_archive.py, which parses the file without the crate that
wrote it -- a black-box proof that imported its own subject would pass by
construction -- and it is listed in this guard by name.
"""


def tracked_sources() -> list[Path]:
    """Every Git-tracked text file, as paths relative to the project root.

    Tracked only: a generated artifact or a build output that happens to
    contain the magic is not a second specification of it.
    """
    listing = subprocess.run(
        ["git", "-C", str(PROJECT_ROOT), "ls-files", "-z"],
        capture_output=True,
        check=True,
    ).stdout
    return [Path(name.decode()) for name in listing.split(b"\0") if name]


def format_copies(path: Path, text: str) -> list[str]:
    """Pure predicate over (path, text): facts restated outside their home."""
    if path in (FORMAT, FORMAT_V2) or path in ALLOWLIST:
        return []
    # format.rs's own tests are part of the definition's home.
    if path.parts[:3] == ("crates", "capsem-archive", "src") and "tests" in path.name:
        return []
    if path.parts[:4] == ("crates", "capsem-archive", "src", "format"):
        return []
    if path == Path(__file__).relative_to(PROJECT_ROOT):
        return []
    return [f"{path} defines `{fact}`" for fact, pattern in FORMAT_FACTS if pattern.search(text)]


def test_the_format_is_where_this_guard_says() -> None:
    """A guard over a file nobody has asserts nothing."""
    home = PROJECT_ROOT / FORMAT
    assert home.is_file(), f"{FORMAT} is missing; this guard is vacuous"
    text = home.read_text()
    for fact, pattern in FORMAT_FACTS:
        assert pattern.search(text), f"{FORMAT} no longer defines `{fact}`; this guard is vacuous"


def test_the_allowlisted_copy_still_exists_and_still_needs_to() -> None:
    """An allowlist entry for a file nobody has is a hole, not an exemption."""
    for path, reason in ALLOWLIST.items():
        candidate = PROJECT_ROOT / path
        assert candidate.is_file(), f"{path} is gone; remove its allowlist entry"
        assert reason, f"{path} is allowlisted without a reason"
        copy = candidate.read_text()
        assert any(pattern.search(copy) for _, pattern in FORMAT_FACTS), (
            f"{path} no longer restates the format; it does not need an exemption"
        )


def test_the_format_is_defined_in_exactly_one_place() -> None:
    violations: list[str] = []
    for path in tracked_sources():
        candidate = PROJECT_ROOT / path
        if candidate.is_symlink() or not candidate.is_file():
            continue
        try:
            text = candidate.read_text()
        except (UnicodeDecodeError, OSError):
            continue
        violations.extend(format_copies(path, text))

    assert not violations, FORMAT_IS_ONE_PLACE_RATIONALE + "\n" + "\n".join(violations)


def test_the_predicate_flags_a_second_definition() -> None:
    """The adversarial case: a reader that decided to know the layout itself."""
    forked = '''
const FILE_MAGIC: &[u8; 8] = b"CAPSEMBL";
const BLOCK_HEADER_BYTES: usize = 40;
'''
    found = format_copies(Path("crates/capsem-service/src/body_routes.rs"), forked)
    assert len(found) == 2, found
    assert any("CAPSEMBL" in line for line in found), found
    assert any("BLOCK_HEADER_BYTES" in line for line in found), found
    # A prefix is not a disguise.
    prefixed = format_copies(Path("tests/ironbank/t.py"), "ARCHIVE_FILE_HEADER_BYTES = 16\n")
    assert len(prefixed) == 1, prefixed


def test_the_predicate_allows_the_home_and_the_named_proof() -> None:
    """format.rs defines them; the black-box helper is allowed to restate."""
    text = 'FILE_MAGIC = b"CAPSEMBL"\nBLOCK_MAGIC = b"BLK1"\nFILE_HEADER_BYTES = 16\nBLOCK_HEADER_BYTES = 44\n'
    assert format_copies(FORMAT, text) == []
    assert format_copies(Path("tests/helpers/body_archive.py"), text) == []
    # An unnamed second helper is not covered by the named one's reason.
    assert len(format_copies(Path("tests/helpers/other_archive.py"), text)) == 4


def test_using_the_constants_is_not_defining_them() -> None:
    """The whole point of their being public: a caller imports and uses them."""
    caller = """
use capsem_archive::{BLOCK_HEADER_BYTES, FILE_HEADER_BYTES};

fn first_block(bytes: &[u8]) -> &[u8] {
    let body = &bytes[FILE_HEADER_BYTES..];
    &body[BLOCK_HEADER_BYTES..]
}
"""
    assert format_copies(Path("crates/capsem-service/src/body_routes.rs"), caller) == []
    assert format_copies(Path("tests/ironbank/test_stats_detail_contract.py"),
                         "ARCHIVE_FIRST_BLOCK_OFFSET = FILE_HEADER_BYTES\n") == []
