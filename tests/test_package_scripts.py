"""Fast contract tests for package assembly scripts.

These tests shadow platform packaging tools with tiny fakes so the shell
scripts can be exercised on any developer machine. The fakes inspect the
temporary payload before the script's trap deletes it.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path


REPO_ROOT = Path(__file__).parent.parent
BUILD_PKG = REPO_ROOT / "scripts" / "build-pkg.sh"
REPACK_DEB = REPO_ROOT / "scripts" / "repack-deb.sh"
PKG_POSTINSTALL = REPO_ROOT / "scripts" / "pkg-scripts" / "postinstall"
DEB_POSTINST = REPO_ROOT / "scripts" / "deb-postinst.sh"

HOST_BINARIES = [
    "capsem",
    "capsem-service",
    "capsem-process",
    "capsem-mcp",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
    "capsem-gateway",
    "capsem-tray",
]


def _seed_host_binaries(bin_dir: Path) -> None:
    bin_dir.mkdir(parents=True)
    for name in HOST_BINARIES:
        binary = bin_dir / name
        binary.write_text(f"#!/bin/sh\necho {name}\n")
        binary.chmod(0o755)


def _seed_assets(assets_dir: Path) -> None:
    assets_dir.mkdir(parents=True)
    (assets_dir / "manifest.json").write_text('{"format":2}\n')
    (assets_dir / "manifest.json.minisig").write_text("trusted comment: test\nsig\n")
    (assets_dir / "manifest-sign.dev.pub").write_text("untrusted comment: dev pub\nPUB\n")


def _fake_tool_dir(tmp_path: Path) -> Path:
    tool_dir = tmp_path / "fake-tools"
    tool_dir.mkdir()
    return tool_dir


def test_build_pkg_payload_includes_signed_manifest_and_helpers(tmp_path):
    """The macOS pkg payload must include manifest.json, minisig, and all helpers."""
    app = tmp_path / "Capsem.app"
    (app / "Contents").mkdir(parents=True)
    (app / "Contents" / "Info.plist").write_text("<plist />\n")
    bin_dir = tmp_path / "bin"
    assets_dir = tmp_path / "assets"
    _seed_host_binaries(bin_dir)
    _seed_assets(assets_dir)

    tool_dir = _fake_tool_dir(tmp_path)
    (tool_dir / "pkgbuild").write_text(
        """#!/bin/sh
set -eu
root=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --root) root="$2"; shift 2 ;;
    --scripts|--identifier|--version) shift 2 ;;
    *) out="$1"; shift ;;
  esac
done
test -f "$root/usr/local/share/capsem/assets/manifest.json"
test -f "$root/usr/local/share/capsem/assets/manifest.json.minisig"
test -f "$root/usr/local/share/capsem/assets/manifest-sign.dev.pub"
test -f "$root/usr/local/share/capsem/Capsem.app/Contents/Info.plist"
for bin in capsem capsem-service capsem-process capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray; do
  test -x "$root/usr/local/share/capsem/bin/$bin"
done
mkdir -p "$(dirname "$out")"
printf pkg > "$out"
"""
    )
    (tool_dir / "productbuild").write_text(
        """#!/bin/sh
set -eu
out="${@: -1}"
mkdir -p "$(dirname "$out")"
printf product > "$out"
"""
    )
    for tool in ("pkgbuild", "productbuild"):
        (tool_dir / tool).chmod(0o755)

    env = os.environ.copy()
    env["PATH"] = f"{tool_dir}:{env['PATH']}"
    result = subprocess.run(
        [str(BUILD_PKG), str(app), str(bin_dir), str(assets_dir), "1.1.0"],
        cwd=tmp_path,
        env=env,
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert result.returncode == 0, result.stdout + result.stderr


def test_repack_deb_copies_signed_manifest_and_all_helpers(tmp_path):
    """The Linux deb repack must add signed manifest files and all host helpers."""
    input_deb = tmp_path / "capsem.deb"
    input_deb.write_text("fixture\n")
    bin_dir = tmp_path / "bin"
    assets_dir = tmp_path / "assets"
    output_deb = tmp_path / "out.deb"
    _seed_host_binaries(bin_dir)
    _seed_assets(assets_dir)

    tool_dir = _fake_tool_dir(tmp_path)
    (tool_dir / "dpkg-deb").write_text(
        """#!/bin/sh
set -eu
case "$1" in
  -R)
    dest="$3"
    mkdir -p "$dest/DEBIAN" "$dest/usr/share/capsem"
    printf 'Package: capsem\\nVersion: 1.1.0\\nArchitecture: all\\nDescription: test\\n' > "$dest/DEBIAN/control"
    ;;
  -b)
    root="$2"
    out="$3"
    test -f "$root/usr/share/capsem/assets/manifest.json"
    test -f "$root/usr/share/capsem/assets/manifest.json.minisig"
    test -f "$root/usr/share/capsem/assets/manifest-sign.dev.pub"
    for bin in capsem capsem-service capsem-process capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray; do
      test -x "$root/usr/bin/$bin"
    done
    printf deb > "$out"
    ;;
  --info)
    printf info\\n
    ;;
  *)
    echo "unexpected dpkg-deb args: $*" >&2
    exit 2
    ;;
esac
"""
    )
    (tool_dir / "dpkg-deb").chmod(0o755)

    env = os.environ.copy()
    env["PATH"] = f"{tool_dir}:{env['PATH']}"
    result = subprocess.run(
        [str(REPACK_DEB), str(input_deb), str(bin_dir), str(assets_dir), str(output_deb)],
        env=env,
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert output_deb.exists()


def test_postinstall_release_critical_commands_fail_loudly():
    """Package postinstall must not hide service registration or setup failures."""
    for script in (PKG_POSTINSTALL, DEB_POSTINST):
        for line in script.read_text().splitlines():
            if "capsem install" in line or "capsem setup" in line:
                assert "|| true" not in line, f"{script}: {line}"
                assert "2>/dev/null" not in line, f"{script}: {line}"


def test_postinstall_seeds_signed_manifest_and_dev_pubkey_loudly():
    """Package postinstall must copy signed manifests before setup verifies them."""
    for script in (PKG_POSTINSTALL, DEB_POSTINST):
        text = script.read_text()
        assert "manifest.json.minisig" in text
        assert "manifest-sign.dev.pub" in text
        assert "install -m 0644" in text
        assert "required package asset missing" in text or "failed to install" in text
        assert 'cp -R "$PKG_SHARE/assets/"*' not in text


def test_macos_postinstall_replaces_symlinked_asset_dir_before_seeding():
    """A local dev symlink must not let root seed package assets into the repo."""
    text = PKG_POSTINSTALL.read_text()

    assert 'if [ -L "$CAPSEM_DIR/assets" ]' in text
    assert 'rm "$CAPSEM_DIR/assets"' in text
    assert text.index('if [ -L "$CAPSEM_DIR/assets" ]') < text.index("# Copy assets")


def test_macos_postinstall_materializes_app_bundle_in_applications():
    """A successful macOS install must leave Capsem.app in /Applications."""
    text = PKG_POSTINSTALL.read_text()

    assert 'src="$PKG_SHARE/Capsem.app"' in text
    assert 'dst="/Applications/Capsem.app"' in text
    assert "ditto \"$src\" \"$dst\"" in text
    assert "required app bundle missing" in text
    assert text.index("install_app_bundle") < text.index("# Copy companion binaries")
