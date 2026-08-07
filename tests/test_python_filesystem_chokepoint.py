"""Hardlinking from Python goes through one audited place.

The Rust sibling of this contract is `test_rust_filesystem_chokepoint.py`, and
it exists because `capsem-admin` staged profile payloads with a hardlink and
put 48 checked-in `config/` files inside published release output -- one inode
each, so a `chmod` on the artifact rewrote tracked source and no content digest
noticed.

Python has exactly one hardlink today and it happens to be safe: both sides of
`builder/docker.py`'s asset alias are build output. "Happens to be safe" is not
a guarantee, and the next one will be written by someone who has not read this
paragraph. `auditfs.stage` classifies before it chooses; this refuses the raw
call anywhere else.

Scoped to `os.link` and not to filesystem mutation generally, for the reason
the Rust version gives: the first version of that guard found 259 call sites,
most of them runtime service code removing its own sockets, and narrowing
afterwards to whatever passed would have been a guard shaped around its own
result. A hardlink is rare, so auditing every one is cheap and closes the class.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]

#: `auditfs` is the audited place; `interception` proxies primitives and must
#: name the ones it proxies.
_ALLOWED = {"auditfs.py", "interception.py"}

_HARDLINK = re.compile(r"\bos\.link\s*\(")


def _sources() -> list[Path]:
    return sorted(
        path
        for path in (PROJECT_ROOT / "src").rglob("*.py")
        if path.name not in _ALLOWED
    )


def test_no_module_hardlinks_outside_the_audited_place() -> None:
    """A raw `os.link` cannot know whether its source is checked in."""
    offenders = [
        f"{path.relative_to(PROJECT_ROOT)}:{index}"
        for path in _sources()
        for index, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1)
        if _HARDLINK.search(line)
    ]
    assert not offenders, (
        f"these hardlink without classifying the source: {offenders}. Use "
        "`capsem.gate.auditfs.stage`, which copies anything git tracks so "
        "published output cannot alias the working tree."
    )


def test_a_tracked_file_is_copied_not_linked(tmp_path: Path) -> None:
    """The defect itself, as an inode comparison."""
    from capsem.gate import auditfs

    repo = tmp_path / "repo"
    (repo / "config").mkdir(parents=True)
    source = repo / "config" / "profile.toml"
    source.write_text("x", encoding="utf-8")

    import subprocess

    for argv in (["git", "init", "--quiet"], ["git", "add", "config/profile.toml"]):
        subprocess.run(argv, cwd=repo, check=True, capture_output=True)

    published = tmp_path / "out" / "profile.toml"
    auditfs.stage(source, published)

    assert published.read_text(encoding="utf-8") == "x"
    assert published.stat().st_ino != source.stat().st_ino, (
        "a checked-in file was hardlinked into published output, so a chmod "
        "there rewrites tracked source and no content digest notices"
    )


def test_build_output_is_linked(tmp_path: Path) -> None:
    """Untracked output still shares an inode; that is what makes it cheap."""
    from capsem.gate import auditfs

    repo = tmp_path / "repo"
    repo.mkdir()
    import subprocess

    subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True, capture_output=True)
    source = repo / "artifact.bin"
    source.write_bytes(b"payload")

    published = tmp_path / "out" / "artifact.bin"
    auditfs.stage(source, published)

    assert published.stat().st_ino == source.stat().st_ino


def test_an_unclassifiable_source_is_copied(tmp_path: Path) -> None:
    """Fails closed: "cannot tell" must not take the linking branch."""
    from capsem.gate import auditfs

    source = tmp_path / "loose.bin"
    source.write_bytes(b"payload")

    published = tmp_path / "out" / "loose.bin"
    auditfs.stage(source, published)

    assert published.read_bytes() == b"payload"
