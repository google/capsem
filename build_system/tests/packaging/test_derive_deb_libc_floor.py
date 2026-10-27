"""Tests for build_system/packaging/linux/derive-deb-libc-floor.py.

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
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPT = REPO_ROOT / "build_system" / "packaging" / "linux" / "derive-deb-libc-floor.py"
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
    header = bytearray(64)
    header[:8] = derive.ELF_MAGIC + struct.pack("<4B", 2, 1, 1, 0)
    header[16:20] = b"\x03\x00\xb7\x00"
    path.write_bytes(header + body)
    path.chmod(0o755)


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


def test_candidate_predepends_uses_actual_symbol_mapping_and_preserves_other_fields(tmp_path, monkeypatch):
    _write_elf(tmp_path / "usr/bin/capsem-ledger", "GLIBC_2.39")
    _write_elf(tmp_path / "usr/bin/gui", "GLIBC_2.99")
    control = tmp_path / "DEBIAN/control"
    control.parent.mkdir()
    original = "Package: capsem\nArchitecture: arm64\nDepends: gui-lib (>= 9)\nPre-Depends: original-tool\nDescription: original\n"
    control.write_text(original)

    def mapping(argv, **kwargs):
        assert "-dPre-Depends" in argv
        assert kwargs["env"]["DEB_HOST_ARCH"] == "arm64"
        context = Path(kwargs["cwd"])
        selected = context / "debian/capsem/usr/bin/capsem-ledger"
        assert argv[-1] == f"-e{selected}"
        assert selected.resolve() == tmp_path / "usr/bin/capsem-ledger"
        assert "Source: capsem\n" in (context / "debian/control").read_text()
        return subprocess.CompletedProcess(argv, 0, "shlibs:Pre-Depends=libc6 (>= 2.39), libgcc-s1 (>= 4.2)\n", "")

    monkeypatch.setattr(derive.subprocess, "run", mapping)
    derive.write_preinstall_dependencies(tmp_path)
    result = control.read_text()
    assert "procps" in result.split("Pre-Depends:", 1)[1].split("\n", 1)[0]
    assert "Depends: gui-lib (>= 9)\n" in result
    assert "Pre-Depends: original-tool, libc6 (>= 2.39), libgcc-s1 (>= 4.2), procps\n" in result
    assert "2.99" not in result
    assert result.startswith("Package: capsem\nArchitecture: arm64\n")
    assert result.endswith("Description: original\n")
    derive.write_preinstall_dependencies(tmp_path)
    assert control.read_text() == result


@pytest.mark.parametrize("output", ["", "shlibs:Depends=libc6 (>= 2.39)\n", "shlibs:Pre-Depends=libc6 (>= 2.38)\n", "shlibs:Pre-Depends=libc6 (>= 2.39)\nextra\n"])
def test_unproven_preinstall_dependency_mapping_preserves_original(tmp_path, monkeypatch, output):
    _write_elf(tmp_path / "usr/bin/capsem-ledger", "GLIBC_2.39")
    control = tmp_path / "DEBIAN/control"
    control.parent.mkdir()
    control.write_text("Package: capsem\nArchitecture: arm64\nDepends: original\n")
    original = control.read_bytes()
    monkeypatch.setattr(derive.subprocess, "run", lambda argv, **kw: subprocess.CompletedProcess(argv, 0, output, ""))
    with pytest.raises(SystemExit, match=r"candidate|mapping"):
        derive.write_preinstall_dependencies(tmp_path)
    assert control.read_bytes() == original


@pytest.mark.parametrize("architecture,machine", [("arm64", 183), ("amd64", 62)])
def test_candidate_control_binds_dpkg_target_instead_of_ambient_host(tmp_path, monkeypatch, architecture, machine):
    binary = tmp_path / "usr/bin/capsem-ledger"
    _write_elf(binary, "GLIBC_2.39")
    data = bytearray(binary.read_bytes())
    data[18:20] = machine.to_bytes(2, "little")
    binary.write_bytes(data)
    control = tmp_path / "DEBIAN/control"
    control.parent.mkdir()
    control.write_text(f"Package: capsem\nArchitecture: {architecture}\nDepends: gui\n")
    monkeypatch.setenv("DEB_HOST_ARCH", "foreign-ambient-host")

    def mapping(argv, **kwargs):
        assert kwargs["env"]["DEB_HOST_ARCH"] == architecture
        assert "--ignore-missing-info" not in argv
        return subprocess.CompletedProcess(argv, 0, "shlibs:Pre-Depends=libc6 (>= 2.39), libgcc-s1 (>= 4.2)\n", "")

    monkeypatch.setattr(derive.subprocess, "run", mapping)
    derive.write_preinstall_dependencies(tmp_path)
    assert "Pre-Depends: libc6 (>= 2.39), libgcc-s1 (>= 4.2)" in control.read_text()
    control.write_text(f"Package: capsem\nArchitecture: {'amd64' if architecture == 'arm64' else 'arm64'}\n")
    with pytest.raises(ValueError, match="matching target"):
        derive.write_preinstall_dependencies(tmp_path)


def test_actual_missing_symbol_information_refuses_before_control_write(tmp_path, monkeypatch):
    _write_elf(tmp_path / "usr/bin/capsem-ledger", "GLIBC_2.39")
    control = tmp_path / "DEBIAN/control"
    control.parent.mkdir()
    control.write_text("Package: capsem\nArchitecture: arm64\nPre-Depends: preserved (>= 10)\n")
    original = control.read_bytes()

    def missing(argv, **kwargs):
        raise subprocess.CalledProcessError(2, argv, stderr="missing actual target symbols")

    monkeypatch.setattr(derive.subprocess, "run", missing)
    with pytest.raises(SystemExit, match="missing actual target symbols"):
        derive.write_preinstall_dependencies(tmp_path)
    assert control.read_bytes() == original


def test_candidate_predepends_resolves_after_strip_before_embed_or_package_output() -> None:
    source = (SCRIPT.parent / "repack-deb.sh").read_text()
    assert source.index("strip_packaged_binaries\n") < source.index("--preinstall-control")
    assert source.index("--preinstall-control") < source.index('embed-preinstall-resource.py" embed')
    assert source.index("--preinstall-control") < source.index('dpkg-deb -b "$WORK_DIR/deb"')


@pytest.mark.parametrize("existing", ["", "Pre-Depends: libc6 (>= 2.40), other-tool | alternate\n"])
def test_predepends_appends_without_weakening_existing_constraints(tmp_path, monkeypatch, existing):
    _write_elf(tmp_path / "usr/bin/capsem-ledger", "GLIBC_2.39")
    control = tmp_path / "DEBIAN/control"
    control.parent.mkdir()
    control.write_text(f"Package: capsem\nArchitecture: arm64\nDepends: untouched\n{existing}")
    monkeypatch.setattr(derive.subprocess, "run", lambda argv, **kw: subprocess.CompletedProcess(argv, 0, "shlibs:Pre-Depends=libc6 (>= 2.39), libgcc-s1 (>= 4.2)\n", ""))
    derive.write_preinstall_dependencies(tmp_path)
    result = control.read_text()
    assert "Depends: untouched\n" in result
    assert "libgcc-s1 (>= 4.2)" in result
    if existing:
        assert "libc6 (>= 2.40), other-tool | alternate" in result


def test_missing_dpkg_tool_does_not_publish_control(tmp_path, monkeypatch):
    _write_elf(tmp_path / "usr/bin/capsem-ledger", "GLIBC_2.39")
    control = tmp_path / "DEBIAN/control"
    control.parent.mkdir()
    control.write_text("Package: capsem\nArchitecture: arm64\nDepends: unchanged\n")
    original = control.read_bytes()

    def missing(*args, **kwargs):
        raise FileNotFoundError("dpkg-shlibdeps missing")

    monkeypatch.setattr(derive.subprocess, "run", missing)
    with pytest.raises(SystemExit, match="dpkg-shlibdeps missing"):
        derive.write_preinstall_dependencies(tmp_path)
    assert control.read_bytes() == original


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


@pytest.mark.skipif(sys.platform != "linux" or shutil.which("dpkg-shlibdeps") is None, reason="needs native Debian symbol mappings")
def test_predepends_generation_uses_builder_stdlib_python(tmp_path):
    """The sealed package helper has no gate-only Python dependencies."""
    binary = tmp_path / "usr/bin/capsem-ledger"
    binary.parent.mkdir(parents=True)
    shutil.copy2(_ELF_SOURCE, binary)
    machine = int.from_bytes(binary.read_bytes()[18:20], "little")
    architecture = {62: "amd64", 183: "arm64"}[machine]
    control = tmp_path / "DEBIAN/control"
    control.parent.mkdir()
    control.write_text(f"Package: capsem\nArchitecture: {architecture}\nDescription: metadata probe\n")
    result = subprocess.run(
        [sys.executable, "-I", "-S", str(SCRIPT), str(tmp_path), "--preinstall-control"],
        text=True, capture_output=True, check=False,
    )
    assert result.returncode == 0, result.stderr
    assert "procps" in control.read_text().split("Pre-Depends:", 1)[1]
