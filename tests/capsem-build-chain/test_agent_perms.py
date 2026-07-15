"""Builder enforces 555 on guest binaries host-side.

The container-native agent build chmods inside the container, but Docker-for-Mac
bind-mount semantics non-deterministically preserve the group/other write bit
on the host. The builder must re-apply 555 on the host so the guest-binary
read-only invariant (CLAUDE.md) holds for every caller.
"""

import pytest
from pathlib import Path


from capsem.builder.docker import GUEST_BINARIES, enforce_guest_binary_perms

pytestmark = pytest.mark.build_chain

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_enforce_guest_binary_perms_sets_555(tmp_path):
    """Bind-mounted outputs become host-owned atomic 0555 files."""
    paths = []
    original_inodes = {}
    for name in GUEST_BINARIES:
        p = tmp_path / name
        p.write_bytes(b"binary")
        p.chmod(0o755)
        paths.append(p)
        original_inodes[p] = p.stat().st_ino

    enforce_guest_binary_perms(paths)

    for p in paths:
        mode = p.stat().st_mode & 0o777
        assert mode == 0o555, f"{p.name} expected 0o555, got {oct(mode)}"
        assert p.read_bytes() == b"binary"
        assert p.stat().st_ino != original_inodes[p]


def test_enforce_guest_binary_perms_idempotent(tmp_path):
    """Already-555 paths stay 555."""
    p = tmp_path / "capsem-pty-agent"
    p.write_bytes(b"binary")
    p.chmod(0o555)

    enforce_guest_binary_perms([p])

    assert p.stat().st_mode & 0o777 == 0o555


def test_enforce_guest_binary_perms_missing_file_raises(tmp_path):
    """Missing path surfaces as an error, not a silent skip."""
    with pytest.raises(FileNotFoundError):
        enforce_guest_binary_perms([tmp_path / "does-not-exist"])


def test_pack_initrd_reasserts_cached_guest_binary_permissions():
    """Cached staging binaries are repaired before initrd packaging."""
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    recipe = justfile.split("_pack-initrd:", 1)[1].split("\n_", 1)[0]

    assert 'chmod 555 "$RELEASE_DIR/$b"' in recipe
