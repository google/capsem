"""Builder enforces 555 on guest binaries host-side.

The container-native agent build chmods inside the container, but Docker-for-Mac
bind-mount semantics non-deterministically preserve the group/other write bit
on the host. The builder must re-apply 555 on the host so the guest-binary
read-only invariant (CLAUDE.md) holds for every caller.
"""

import os
import pytest

from pathlib import Path

from capsem.builder.docker import GUEST_BINARIES, enforce_guest_binary_perms

pytestmark = pytest.mark.build_chain


def test_enforce_guest_binary_perms_sets_555(tmp_path):
    """Paths with 0o755 on disk become 0o555 after enforcement."""
    paths = []
    for name in GUEST_BINARIES:
        p = tmp_path / name
        p.write_bytes(b"binary")
        p.chmod(0o755)
        paths.append(p)

    enforce_guest_binary_perms(paths)

    for p in paths:
        mode = p.stat().st_mode & 0o777
        assert mode == 0o555, f"{p.name} expected 0o555, got {oct(mode)}"


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
