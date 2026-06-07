"""Tests for scripts/repack-deb.sh.

The script is the seam between Tauri's bundler and the host install: if it
silently drops a companion binary or accepts a malformed input path, every
subsequent `dpkg -i` either installs a broken layout or explodes in dpkg
with a cryptic error. None of that is caught by pytest elsewhere because
the whole test-install stage runs inside Docker after a 30-second Tauri
build.

These tests seed a minimal fixture .deb with `dpkg-deb -b` and invoke the
script directly, so the full repack round-trip runs in under a second.
Skipped cleanly on any machine without `dpkg-deb` on PATH (macOS default);
executed in Linux CI and inside the capsem-install-test container.
"""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
SCRIPT = REPO_ROOT / "scripts" / "repack-deb.sh"
POSTINST = REPO_ROOT / "scripts" / "deb-postinst.sh"

REQUIRED_BINARIES = [
    "capsem",
    "capsem-service",
    "capsem-process",
    "capsem-mcp",
    "capsem-gateway",
    "capsem-tray",
]

pytestmark = pytest.mark.skipif(
    shutil.which("dpkg-deb") is None,
    reason="dpkg-deb not on PATH (install on macOS via `brew install dpkg`)",
)


def _build_fixture_deb(workdir: Path, name: str = "capsem-fixture", version: str = "0.0.1") -> Path:
    """Build a minimal valid .deb in workdir. Returns the path to the .deb."""
    root = workdir / f"{name}-src"
    (root / "DEBIAN").mkdir(parents=True)
    (root / "DEBIAN" / "control").write_text(
        f"Package: {name}\n"
        f"Version: {version}\n"
        f"Architecture: all\n"
        f"Maintainer: Test <test@example.com>\n"
        f"Description: fixture package for repack-deb tests\n"
    )
    # Add a token file so the archive isn't empty.
    (root / "usr" / "share" / name).mkdir(parents=True)
    (root / "usr" / "share" / name / "marker.txt").write_text("fixture")
    deb_path = workdir / f"{name}_{version}_all.deb"
    subprocess.run(
        ["dpkg-deb", "-b", str(root), str(deb_path)],
        check=True, capture_output=True,
    )
    return deb_path


def _seed_binaries(bin_dir: Path, which: list[str] = None):
    """Drop fake executable files named like the companion binaries."""
    if which is None:
        which = REQUIRED_BINARIES
    bin_dir.mkdir(parents=True, exist_ok=True)
    for name in which:
        path = bin_dir / name
        # Minimal shell script so the file is non-empty; repack-deb copies
        # bytes, doesn't care about ELF validity.
        path.write_text(f"#!/bin/sh\necho {name} stub\n")
        path.chmod(0o755)


def _run_repack(input_deb: Path, bin_dir: Path, output_deb: Path = None,
                 timeout: int = 30) -> subprocess.CompletedProcess:
    args = [str(SCRIPT), str(input_deb), str(bin_dir)]
    if output_deb is not None:
        args.append(str(output_deb))
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout)


def _deb_contents(deb: Path, dest: Path) -> Path:
    """Extract a .deb to dest/ and return dest."""
    subprocess.run(
        ["dpkg-deb", "-R", str(deb), str(dest)],
        check=True, capture_output=True,
    )
    return dest


def test_happy_path_adds_every_companion_binary(tmp_path):
    """All six companion binaries land in /usr/bin with mode 755."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    _seed_binaries(bin_dir)
    output = tmp_path / "out.deb"

    res = _run_repack(fixture, bin_dir, output)
    assert res.returncode == 0, (
        f"repack-deb.sh failed: stdout={res.stdout!r} stderr={res.stderr!r}"
    )
    assert output.exists(), "output .deb was not created"

    extracted = _deb_contents(output, tmp_path / "extracted")
    for name in REQUIRED_BINARIES:
        binary = extracted / "usr" / "bin" / name
        assert binary.exists(), f"{name} missing from repacked .deb"
        assert binary.stat().st_mode & 0o777 == 0o755, (
            f"{name} installed with mode {oct(binary.stat().st_mode & 0o777)}, expected 0o755"
        )


def test_postinst_script_is_included(tmp_path):
    """DEBIAN/postinst is copied from scripts/deb-postinst.sh and is executable."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    _seed_binaries(bin_dir)
    output = tmp_path / "out.deb"

    res = _run_repack(fixture, bin_dir, output)
    assert res.returncode == 0

    extracted = _deb_contents(output, tmp_path / "extracted")
    postinst = extracted / "DEBIAN" / "postinst"
    assert postinst.exists(), "DEBIAN/postinst not written"
    assert postinst.stat().st_mode & 0o777 == 0o755, (
        f"postinst mode {oct(postinst.stat().st_mode & 0o777)}, expected 0o755"
    )
    # Match on a fragment of the on-disk source so this catches a wrong-file
    # copy, not a text-munging bug.
    expected_head = POSTINST.read_text().splitlines()[0]
    assert postinst.read_text().startswith(expected_head), (
        "postinst doesn't look like scripts/deb-postinst.sh"
    )


def test_missing_companion_binary_fails_loudly(tmp_path):
    """Any missing binary must fail with a clear error naming the file.

    Silent drops would ship an incomplete install.
    """
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    # Omit capsem-tray on purpose.
    _seed_binaries(bin_dir, which=[b for b in REQUIRED_BINARIES if b != "capsem-tray"])

    res = _run_repack(fixture, bin_dir)
    assert res.returncode != 0, (
        "repack should have failed with capsem-tray missing; "
        f"stdout={res.stdout!r} stderr={res.stderr!r}"
    )
    combined = res.stdout + res.stderr
    assert "capsem-tray" in combined, (
        f"error message should mention capsem-tray, got: {combined!r}"
    )


def test_path_with_embedded_newline_fails(tmp_path):
    """Newline-joined paths must fail fast, not corrupt an output .deb.

    Regression for the `just test-install` bug where a persistent
    /cargo-target volume left a stale .deb next to the current one and
    `DEB=$(ls *.deb)` captured both joined by a newline. repack-deb then
    saw one arg containing a literal newline and dpkg-deb bailed -- the
    test pins that behaviour so a future "helpful" fix doesn't silently
    consume the first path and pretend everything's fine.
    """
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    _seed_binaries(bin_dir)

    mangled = f"{fixture}\n{fixture}"
    res = subprocess.run(
        [str(SCRIPT), mangled, str(bin_dir)],
        capture_output=True, text=True, timeout=30,
    )
    assert res.returncode != 0, (
        "repack should have failed on a newline-containing input path; "
        f"stdout={res.stdout!r} stderr={res.stderr!r}"
    )


def test_version_gets_build_timestamp_stamped(tmp_path):
    """DEBIAN/control's Version field gains a numeric suffix so repeat installs see a newer package."""
    fixture = _build_fixture_deb(tmp_path, version="0.0.1")
    bin_dir = tmp_path / "bin"
    _seed_binaries(bin_dir)
    output = tmp_path / "out.deb"

    res = _run_repack(fixture, bin_dir, output)
    assert res.returncode == 0

    extracted = _deb_contents(output, tmp_path / "extracted")
    control = (extracted / "DEBIAN" / "control").read_text()
    version_line = next(
        (l for l in control.splitlines() if l.startswith("Version:")),
        None,
    )
    assert version_line is not None, f"no Version: line in control: {control!r}"
    # Expect the original "0.0.1" plus a dotted numeric build stamp.
    assert version_line.startswith("Version: 0.0.1."), (
        f"Version should be 0.0.1.<ts>, got: {version_line!r}"
    )
    suffix = version_line[len("Version: 0.0.1."):]
    assert suffix.isdigit() and len(suffix) >= 9, (
        f"expected unix-ish timestamp suffix, got: {suffix!r}"
    )


def test_output_defaults_to_overwriting_input(tmp_path):
    """Omitting the output argument overwrites the input .deb in place."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    _seed_binaries(bin_dir)
    original_size = fixture.stat().st_size

    res = _run_repack(fixture, bin_dir)  # no output arg
    assert res.returncode == 0

    # Original .deb path still exists and is now larger (companion binaries added).
    assert fixture.exists()
    assert fixture.stat().st_size > original_size, (
        "in-place repack should produce a larger .deb after adding binaries"
    )
