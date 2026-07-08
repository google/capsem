"""Tests for site/public/install.sh -- OS/arch detection and asset URL selection.

Sources the install script with __INSTALL_SH_SOURCED=1 to access functions
without triggering the main install flow. Uses stub uname to test detection
logic in isolation.
"""

from __future__ import annotations

import subprocess
import textwrap
from pathlib import Path


INSTALL_SH = Path(__file__).parent.parent / "site" / "public" / "install.sh"


def _run_shell(script: str) -> subprocess.CompletedProcess[str]:
    """Run a shell snippet that sources install.sh, returns CompletedProcess."""
    return subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        timeout=10,
    )


def _source_and_run(body: str) -> subprocess.CompletedProcess[str]:
    """Source install.sh (guarded) then run body."""
    script = textwrap.dedent(f"""\
        __INSTALL_SH_SOURCED=1
        . "{INSTALL_SH}"
        {body}
    """)
    return _run_shell(script)


# ---------------------------------------------------------------------------
# detect_os
# ---------------------------------------------------------------------------


class TestDetectOS:
    def test_darwin(self):
        r = _source_and_run(
            'uname() { echo "Darwin"; }; detect_os; echo "$OS"'
        )
        assert r.returncode == 0
        assert r.stdout.strip() == "darwin"

    def test_linux(self):
        r = _source_and_run(
            'uname() { echo "Linux"; }; detect_os; echo "$OS"'
        )
        assert r.returncode == 0
        assert r.stdout.strip() == "linux"

    def test_unsupported_os(self):
        r = _source_and_run(
            'uname() { echo "FreeBSD"; }; detect_os'
        )
        assert r.returncode != 0
        assert "unsupported operating system" in r.stderr

    def test_windows_like(self):
        r = _source_and_run(
            'uname() { echo "MINGW64_NT"; }; detect_os'
        )
        assert r.returncode != 0
        assert "unsupported operating system" in r.stderr


# ---------------------------------------------------------------------------
# detect_arch
# ---------------------------------------------------------------------------


class TestDetectArch:
    def test_linux_x86_64(self):
        r = _source_and_run(
            'OS=linux; uname() { echo "x86_64"; }; detect_arch; echo "$ARCH"'
        )
        assert r.returncode == 0
        assert r.stdout.strip() == "amd64"

    def test_linux_amd64(self):
        r = _source_and_run(
            'OS=linux; uname() { echo "amd64"; }; detect_arch; echo "$ARCH"'
        )
        assert r.returncode == 0
        assert r.stdout.strip() == "amd64"

    def test_linux_aarch64(self):
        r = _source_and_run(
            'OS=linux; uname() { echo "aarch64"; }; detect_arch; echo "$ARCH"'
        )
        assert r.returncode == 0
        assert r.stdout.strip() == "arm64"

    def test_linux_arm64(self):
        r = _source_and_run(
            'OS=linux; uname() { echo "arm64"; }; detect_arch; echo "$ARCH"'
        )
        assert r.returncode == 0
        assert r.stdout.strip() == "arm64"

    def test_darwin_arm64(self):
        r = _source_and_run(
            'OS=darwin; uname() { echo "arm64"; }; detect_arch; echo "$ARCH"'
        )
        assert r.returncode == 0
        assert r.stdout.strip() == "arm64"

    def test_darwin_x86_64_rejected(self):
        r = _source_and_run(
            'OS=darwin; uname() { echo "x86_64"; }; detect_arch'
        )
        assert r.returncode != 0
        assert "macOS requires Apple Silicon" in r.stderr

    def test_linux_riscv_rejected(self):
        r = _source_and_run(
            'OS=linux; uname() { echo "riscv64"; }; detect_arch'
        )
        assert r.returncode != 0
        assert "unsupported architecture" in r.stderr


# ---------------------------------------------------------------------------
# find_asset_url
# ---------------------------------------------------------------------------

# Minimal stable release-channel manifest snippet matching release.capsem.org.
FAKE_RELEASE_MANIFEST = r"""
{
  "packages": [
    {
      "architecture": "arm64",
      "binaries": [],
      "kind": "macos_pkg",
      "name": "Capsem-1.5.0.pkg",
      "platform": "macos",
      "status": "current",
      "url": "https://github.com/google/capsem/releases/download/v1.5.0/Capsem-1.5.0.pkg",
      "version": "1.5.0"
    },
    {
      "architecture": "x86_64",
      "binaries": [],
      "kind": "debian_package",
      "name": "Capsem_1.5.0_amd64.deb",
      "platform": "linux",
      "status": "current",
      "url": "https://github.com/google/capsem/releases/download/v1.5.0/Capsem_1.5.0_amd64.deb",
      "version": "1.5.0"
    },
    {
      "architecture": "arm64",
      "binaries": [],
      "kind": "debian_package",
      "name": "Capsem_1.5.0_arm64.deb",
      "platform": "linux",
      "status": "current",
      "url": "https://github.com/google/capsem/releases/download/v1.5.0/Capsem_1.5.0_arm64.deb",
      "version": "1.5.0"
    }
  ]
}
"""


class TestFindAssetURL:
    def _run(self, os_val: str, arch_val: str) -> subprocess.CompletedProcess[str]:
        # Escape the JSON for shell embedding via a heredoc.
        script = textwrap.dedent(f"""\
            __INSTALL_SH_SOURCED=1
            . "{INSTALL_SH}"
            RELEASE_MANIFEST=$(cat <<'ENDJSON'
{FAKE_RELEASE_MANIFEST}
ENDJSON
            )
            find_asset_url "$RELEASE_MANIFEST" "{os_val}" "{arch_val}"
            printf '%s\n%s\n' "$ASSET_URL" "$ASSET_VERSION"
        """)
        return _run_shell(script)

    def test_darwin_pkg(self):
        """macOS installer downloads the signed/notarized .pkg (DMG dropped)."""
        r = self._run("darwin", "arm64")
        assert r.returncode == 0
        url, version = r.stdout.strip().splitlines()
        assert url.endswith("/Capsem-1.5.0.pkg")
        assert version == "1.5.0"

    def test_linux_amd64_deb(self):
        r = self._run("linux", "amd64")
        assert r.returncode == 0
        url, version = r.stdout.strip().splitlines()
        assert url.endswith("/Capsem_1.5.0_amd64.deb")
        assert version == "1.5.0"

    def test_linux_arm64_deb(self):
        r = self._run("linux", "arm64")
        assert r.returncode == 0
        url, version = r.stdout.strip().splitlines()
        assert url.endswith("/Capsem_1.5.0_arm64.deb")
        assert version == "1.5.0"

    def test_missing_asset_errors(self):
        r = self._run("linux", "s390x")
        assert r.returncode != 0
        assert "no matching asset" in r.stderr


def test_installer_uses_release_channel_manifest_not_github_latest() -> None:
    script = INSTALL_SH.read_text(encoding="utf-8")

    assert "https://release.capsem.org" in script
    assert "/assets/${CAPSEM_CHANNEL}/manifest.json" in script
    assert "api.github.com/repos/${REPO}/releases/latest" not in script
    assert "releases/latest" not in script
