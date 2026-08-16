"""Tests for scripts/derive-deb-libc-floor.py.

The first published Debian package declared `libwebkit2gtk-4.1-0, libgtk-3-0,
libxdo3`
and no libc, so it installed cleanly on glibc older than the binaries needed
and then failed at runtime instead of being refused. The floor is now read out
of the shipped bytes, and these tests pin the two ways that reading can go
quietly wrong: picking the wrong maximum, and finding nothing at all.
"""

import importlib.util
import re
import shutil
import struct
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
SCRIPT = REPO_ROOT / "scripts" / "derive-deb-libc-floor.py"
_ELF_SOURCE = Path("/bin/true")


def _module():
    spec = importlib.util.spec_from_file_location("derive_deb_libc_floor", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


derive = _module()


def _write_elf(path: Path, *versions: str) -> None:
    """A file that starts with the ELF magic and names ``versions`` inside it."""
    path.parent.mkdir(parents=True, exist_ok=True)
    body = b"\x00".join(version.encode() for version in versions)
    path.write_bytes(derive.ELF_MAGIC + struct.pack("<4B", 2, 1, 1, 0) + b"\x00" * 8 + body)


def test_floor_is_the_highest_version_across_every_binary(tmp_path):
    _write_elf(tmp_path / "usr/bin/capsem", "GLIBC_2.17", "GLIBC_2.34")
    _write_elf(tmp_path / "usr/bin/capsem-service", "GLIBC_2.39")
    _write_elf(tmp_path / "usr/bin/capsem-tui", "GLIBC_2.28")

    assert derive.floor_for(tmp_path) == (2, 39)
    assert derive.clause(derive.floor_for(tmp_path), "libc6") == "libc6 (>= 2.39)"


def test_versions_compare_numerically_not_lexically(tmp_path):
    """String order puts 2.9 above 2.39; the package would then be installable."""
    _write_elf(tmp_path / "usr/bin/capsem", "GLIBC_2.9", "GLIBC_2.39")

    assert derive.floor_for(tmp_path) == (2, 39)


def test_three_component_versions_are_kept(tmp_path):
    _write_elf(tmp_path / "usr/bin/capsem", "GLIBC_2.38", "GLIBC_2.38.1")

    assert derive.clause(derive.floor_for(tmp_path), "libc6") == "libc6 (>= 2.38.1)"


def test_non_elf_files_do_not_contribute_a_floor(tmp_path):
    """A shell wrapper mentioning a version must not raise the floor."""
    _write_elf(tmp_path / "usr/bin/capsem", "GLIBC_2.28")
    (tmp_path / "usr/bin/wrapper.sh").write_text("#!/bin/sh\n# needs GLIBC_2.99\n")

    assert derive.floor_for(tmp_path) == (2, 28)


def test_a_longer_symbol_that_merely_contains_a_version_is_not_a_version(tmp_path):
    _write_elf(tmp_path / "usr/bin/capsem", "GLIBC_2.28", "XGLIBC_2.99", "GLIBC_2.991x")

    assert derive.floor_for(tmp_path) == (2, 28)


def test_a_tree_with_no_elf_binaries_fails_loudly(tmp_path):
    """Silence here is what shipped the broken package; it must not return ''."""
    (tmp_path / "usr/share").mkdir(parents=True)
    (tmp_path / "usr/share/notes.txt").write_text("no binaries here")

    with pytest.raises(SystemExit, match="no ELF binaries found"):
        derive.floor_for(tmp_path)


def test_elf_binaries_that_reference_no_glibc_fail_loudly(tmp_path):
    _write_elf(tmp_path / "usr/bin/capsem", "some_other_symbol")

    with pytest.raises(SystemExit, match="reference glibc"):
        derive.floor_for(tmp_path)


@pytest.mark.skipif(not _ELF_SOURCE.is_file(), reason="no /bin/true to read")
def test_derived_floor_matches_objdump_on_a_real_binary(tmp_path):
    """The regex read of .dynstr must agree with the authoritative tool."""
    if shutil.which("objdump") is None:
        pytest.skip("objdump not on PATH")
    shutil.copy2(_ELF_SOURCE, tmp_path / "true")

    symbols = subprocess.run(
        ["objdump", "-T", str(_ELF_SOURCE)], capture_output=True, text=True, check=True
    ).stdout
    found = re.findall(r"GLIBC_(\d+)\.(\d+)(?:\.(\d+))?", symbols)
    if not found:
        pytest.skip(f"{_ELF_SOURCE} references no versioned glibc symbols")
    expected = max(tuple(int(part) for part in groups if part) for groups in found)

    assert derive.floor_for(tmp_path) == expected
