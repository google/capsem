"""Tests for site/public/install.sh -- OS/arch detection and asset URL selection.

Sources the install script with __INSTALL_SH_SOURCED=1 to access functions
without triggering the main install flow. Uses stub uname to test detection
logic in isolation.
"""

from __future__ import annotations

import os
import subprocess
import textwrap
from pathlib import Path


INSTALL_SH = Path(__file__).parent.parent / "site" / "public" / "install.sh"
DOCS_INSTALL_SH = Path(__file__).parent.parent / "docs" / "public" / "install.sh"


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


def test_published_installer_copies_cannot_drift() -> None:
    """Both publish trees must serve the same fail-closed installer contract."""
    assert DOCS_INSTALL_SH.read_bytes() == INSTALL_SH.read_bytes()


# ---------------------------------------------------------------------------
# detect_os
# ---------------------------------------------------------------------------


class TestDetectOS:
    def test_darwin(self):
        r = _source_and_run('uname() { echo "Darwin"; }; detect_os; echo "$OS"')
        assert r.returncode == 0
        assert r.stdout.strip() == "darwin"

    def test_linux(self):
        r = _source_and_run('uname() { echo "Linux"; }; detect_os; echo "$OS"')
        assert r.returncode == 0
        assert r.stdout.strip() == "linux"

    def test_unsupported_os(self):
        r = _source_and_run('uname() { echo "FreeBSD"; }; detect_os')
        assert r.returncode != 0
        assert "unsupported operating system" in r.stderr

    def test_windows_like(self):
        r = _source_and_run('uname() { echo "MINGW64_NT"; }; detect_os')
        assert r.returncode != 0
        assert "unsupported operating system" in r.stderr


# ---------------------------------------------------------------------------
# detect_arch
# ---------------------------------------------------------------------------


class TestDetectArch:
    def test_linux_x86_64(self):
        r = _source_and_run('OS=linux; uname() { echo "x86_64"; }; detect_arch; echo "$ARCH"')
        assert r.returncode == 0
        assert r.stdout.strip() == "amd64"

    def test_linux_amd64(self):
        r = _source_and_run('OS=linux; uname() { echo "amd64"; }; detect_arch; echo "$ARCH"')
        assert r.returncode == 0
        assert r.stdout.strip() == "amd64"

    def test_linux_aarch64(self):
        r = _source_and_run('OS=linux; uname() { echo "aarch64"; }; detect_arch; echo "$ARCH"')
        assert r.returncode == 0
        assert r.stdout.strip() == "arm64"

    def test_linux_arm64(self):
        r = _source_and_run('OS=linux; uname() { echo "arm64"; }; detect_arch; echo "$ARCH"')
        assert r.returncode == 0
        assert r.stdout.strip() == "arm64"

    def test_darwin_arm64(self):
        r = _source_and_run('OS=darwin; uname() { echo "arm64"; }; detect_arch; echo "$ARCH"')
        assert r.returncode == 0
        assert r.stdout.strip() == "arm64"

    def test_darwin_x86_64_rejected(self):
        r = _source_and_run('OS=darwin; uname() { echo "x86_64"; }; detect_arch')
        assert r.returncode != 0
        assert "macOS requires Apple Silicon" in r.stderr

    def test_linux_riscv_rejected(self):
        r = _source_and_run('OS=linux; uname() { echo "riscv64"; }; detect_arch')
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
      "binaries": [
        {
          "digest": {"sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
          "name": "capsem"
        }
      ],
      "bytes": 20,
      "digest": {"sha256": "d2145fea950ac94a3fe793b2d555212e254d9d8863dbccbe1cd07af49a7cae48"},
      "kind": "macos_pkg",
      "name": "Capsem-1.5.0.pkg",
      "platform": "macos",
      "status": "current",
      "url": "https://github.com/google/capsem/releases/download/v1.5.0/Capsem-1.5.0.pkg",
      "version": "1.5.0"
    },
    {
      "architecture": "amd64",
      "binaries": [],
      "bytes": 17,
      "digest": {"sha256": "f46ed56922b881235bea694d06c5e366954607fd25526c9dbf56ffefa67830b9"},
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
      "bytes": 17,
      "digest": {"sha256": "f46ed56922b881235bea694d06c5e366954607fd25526c9dbf56ffefa67830b9"},
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

    def test_package_integrity_comes_from_package_not_nested_binary(self):
        script = textwrap.dedent(f"""\
            __INSTALL_SH_SOURCED=1
            . "{INSTALL_SH}"
            RELEASE_MANIFEST=$(cat <<'ENDJSON'
{FAKE_RELEASE_MANIFEST}
ENDJSON
            )
            find_asset_url "$RELEASE_MANIFEST" darwin arm64
            printf '%s\n%s\n' "$ASSET_BYTES" "$ASSET_SHA256"
        """)

        result = _run_shell(script)

        assert result.returncode == 0, result.stderr
        assert result.stdout.strip().splitlines() == [
            "20",
            "d2145fea950ac94a3fe793b2d555212e254d9d8863dbccbe1cd07af49a7cae48",
        ]

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


def test_macos_installers_apply_pkg_before_cleaning_download(tmp_path: Path) -> None:
    """The curl installer must synchronously apply the pkg, not hand off a temp file."""
    for install_script in (INSTALL_SH, DOCS_INSTALL_SH):
        install_root = tmp_path / install_script.parents[1].name
        call_log = install_root / "installer-call"
        script = textwrap.dedent(f"""\
            __INSTALL_SH_SOURCED=1
            . "{install_script}"
            mktemp() {{
                mkdir -p "{install_root}/download"
                printf '%s\\n' "{install_root}/download"
            }}
            curl() {{
                _output=''
                while [ "$#" -gt 0 ]; do
                    if [ "$1" = '-o' ]; then
                        shift
                        _output="$1"
                    fi
                    shift
                done
                test -n "$_output"
                printf 'fake signed package\\n' > "$_output"
            }}
            open() {{
                echo 'unexpected GUI installer handoff' >&2
                return 97
            }}
            sudo() {{
                case "$1" in
                    /usr/bin/install)
                        if [ "$2" = '-d' ]; then
                            test "$3" = '-o'
                            test "$4" = 'root'
                            test "$5" = '-g'
                            test "$6" = 'wheel'
                            test "$7" = '-m'
                            test "$8" = '0700'
                        else
                            test "$2" = '-o'
                            test "$3" = 'root'
                            test "$4" = '-g'
                            test "$5" = 'wheel'
                            test "$6" = '-m'
                            test "$7" = '0600'
                            test -f "$8"
                        fi
                        ;;
                    /usr/sbin/installer)
                        printf '%s\\n' "$*" > "{call_log}"
                        test "$2" = '-pkg'
                        test -f "$3"
                        test "$4" = '-target'
                        test "$5" = '/'
                        ;;
                    rm)
                        test "$2" = '-f'
                        ;;
                    *) return 98 ;;
                esac
            }}
            install_macos \
              'https://example.invalid/Capsem.pkg' \
              '1.5.0' \
              '20' \
              'd2145fea950ac94a3fe793b2d555212e254d9d8863dbccbe1cd07af49a7cae48' \
              'Capsem-1.5.0.pkg'
        """)

        result = _run_shell(script)

        assert result.returncode == 0, result.stderr
        assert call_log.read_text(encoding="utf-8").strip() == (
            f"/usr/sbin/installer -pkg {install_root}/download/Capsem.pkg -target /"
        )
        assert not (install_root / "download").exists()
        assert "Capsem 1.5.0 installed." in result.stdout


def test_public_macos_installer_hands_package_a_secure_target_user() -> None:
    for install_script in (INSTALL_SH, DOCS_INSTALL_SH):
        source = install_script.read_text(encoding="utf-8")
        assert "prepare_macos_install_user" in source
        assert "CAPSEM_INSTALL_USER_REQUEST" in source
        assert "/usr/bin/install -d -o root -g wheel -m 0700" in source
        assert "/usr/bin/install -o root -g wheel -m 0600" in source
        assert "clear_macos_install_user_request" in source


def test_macos_curl_pipe_entrypoint_installs_pkg_end_to_end(tmp_path: Path) -> None:
    """Exercise the public script through stdin, matching `curl ... | sh`."""
    bin_dir = tmp_path / "bin"
    temp_dir = tmp_path / "tmp"
    bin_dir.mkdir()
    temp_dir.mkdir()
    manifest = tmp_path / "manifest.json"
    manifest.write_text(FAKE_RELEASE_MANIFEST, encoding="utf-8")
    installer_log = tmp_path / "installer-call"

    commands = {
        "uname": """#!/bin/sh
case "$1" in
    -s) echo Darwin ;;
    -m) echo arm64 ;;
    *) exit 2 ;;
esac
""",
        "sw_vers": """#!/bin/sh
test "$1" = '-productVersion'
echo 14.7.5
""",
        "curl": """#!/bin/sh
output=''
while [ "$#" -gt 0 ]; do
    if [ "$1" = '-o' ]; then
        shift
        output="$1"
    fi
    shift
done
if [ -n "$output" ]; then
    printf 'fake signed package\n' > "$output"
else
    /bin/cat "$FAKE_MANIFEST"
fi
""",
        "open": """#!/bin/sh
echo 'unexpected GUI installer handoff' >&2
exit 97
""",
        "sudo": """#!/bin/sh
case "$1" in
    /usr/bin/install)
        if [ "$2" = '-d' ]; then
            test "$3" = '-o'
            test "$4" = 'root'
            test "$5" = '-g'
            test "$6" = 'wheel'
            test "$7" = '-m'
            test "$8" = '0700'
        else
            test "$2" = '-o'
            test "$3" = 'root'
            test "$4" = '-g'
            test "$5" = 'wheel'
            test "$6" = '-m'
            test "$7" = '0600'
            test -f "$8"
        fi
        ;;
    /usr/sbin/installer)
        printf '%s\n' "$*" > "$INSTALLER_LOG"
        test "$2" = '-pkg'
        test -f "$3"
        test "$4" = '-target'
        test "$5" = '/'
        ;;
    rm)
        test "$2" = '-f'
        ;;
    *) exit 98 ;;
esac
""",
    }
    for name, body in commands.items():
        command = bin_dir / name
        command.write_text(body, encoding="utf-8")
        command.chmod(0o755)

    env = os.environ.copy()
    env.update(
        {
            "FAKE_MANIFEST": str(manifest),
            "INSTALLER_LOG": str(installer_log),
            "PATH": f"{bin_dir}:/usr/bin:/bin:/usr/sbin",
            "TMPDIR": str(temp_dir),
        }
    )
    result = subprocess.run(
        ["/bin/sh"],
        input=INSTALL_SH.read_text(encoding="utf-8"),
        capture_output=True,
        text=True,
        timeout=10,
        env=env,
    )

    assert result.returncode == 0, result.stderr
    assert "Installing Capsem 1.5.0 from stable..." in result.stdout
    assert "Capsem 1.5.0 installed." in result.stdout
    installer_args = installer_log.read_text(encoding="utf-8").strip().split()
    assert installer_args[:2] == ["/usr/sbin/installer", "-pkg"]
    assert installer_args[3:] == ["-target", "/"]
    assert installer_args[2].endswith("/Capsem.pkg")
    assert not Path(installer_args[2]).exists()
    assert list(temp_dir.iterdir()) == []


def test_linux_curl_pipe_entrypoint_installs_verified_deb_end_to_end(tmp_path: Path) -> None:
    """Linux CI's public path must download, verify, and apt-install the selected deb."""
    bin_dir = tmp_path / "bin"
    temp_dir = tmp_path / "tmp"
    bin_dir.mkdir()
    temp_dir.mkdir()
    manifest = tmp_path / "manifest.json"
    manifest.write_text(FAKE_RELEASE_MANIFEST, encoding="utf-8")
    installer_log = tmp_path / "installer-call"

    commands = {
        "uname": """#!/bin/sh
case "$1" in
    -s) echo Linux ;;
    -m) echo x86_64 ;;
    *) exit 2 ;;
esac
""",
        "apt": """#!/bin/sh
exit 99
""",
        "curl": """#!/bin/sh
output=''
while [ "$#" -gt 0 ]; do
    if [ "$1" = '-o' ]; then
        shift
        output="$1"
    fi
    shift
done
if [ -n "$output" ]; then
    printf 'fake deb package\n' > "$output"
else
    /bin/cat "$FAKE_MANIFEST"
fi
""",
        "sudo": """#!/bin/sh
printf '%s\n' "$*" > "$INSTALLER_LOG"
test "$1" = 'apt'
test "$2" = 'install'
test "$3" = '-y'
test -f "$4"
""",
    }
    for name, body in commands.items():
        command = bin_dir / name
        command.write_text(body, encoding="utf-8")
        command.chmod(0o755)

    env = os.environ.copy()
    env.update(
        {
            "FAKE_MANIFEST": str(manifest),
            "INSTALLER_LOG": str(installer_log),
            "PATH": f"{bin_dir}:/usr/bin:/bin:/usr/sbin",
            "TMPDIR": str(temp_dir),
        }
    )
    result = subprocess.run(
        ["/bin/sh"],
        input=INSTALL_SH.read_text(encoding="utf-8"),
        capture_output=True,
        text=True,
        timeout=10,
        env=env,
    )

    assert result.returncode == 0, result.stderr
    assert "Installing Capsem 1.5.0 from stable..." in result.stdout
    assert "Verified Capsem_1.5.0_amd64.deb" in result.stdout
    installer_args = installer_log.read_text(encoding="utf-8").strip().split()
    assert installer_args[:3] == ["apt", "install", "-y"]
    assert installer_args[3].endswith("/capsem.deb")
    assert not Path(installer_args[3]).exists()
    assert list(temp_dir.iterdir()) == []


def test_package_integrity_failure_never_invokes_native_installer(tmp_path: Path) -> None:
    """Corrupt or truncated package bytes must fail before sudo on both platforms."""
    for function_name, filename in (
        ("install_macos", "Capsem.pkg"),
        ("install_linux", "capsem.deb"),
    ):
        sudo_marker = tmp_path / f"{function_name}-sudo-called"
        script = textwrap.dedent(f"""\
            __INSTALL_SH_SOURCED=1
            . "{INSTALL_SH}"
            mktemp() {{
                _dir="{tmp_path}/{function_name}-download"
                mkdir -p "$_dir"
                printf '%s\n' "$_dir"
            }}
            curl() {{
                _output=''
                while [ "$#" -gt 0 ]; do
                    if [ "$1" = '-o' ]; then shift; _output="$1"; fi
                    shift
                done
                printf 'corrupt\n' > "$_output"
            }}
            sudo() {{ touch "{sudo_marker}"; return 0; }}
            {function_name} 'https://example.invalid/{filename}' '1.5.0' '999' \
              'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' \
              '{filename}'
        """)

        result = _run_shell(script)

        assert result.returncode != 0
        assert "integrity check failed" in result.stderr
        assert not sudo_marker.exists()
