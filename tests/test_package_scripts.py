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
PREPARE_ADMIN_CLI = REPO_ROOT / "scripts" / "prepare-admin-cli.sh"
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
    "capsem-admin",
]


def _seed_host_binaries(bin_dir: Path) -> None:
    bin_dir.mkdir(parents=True)
    for name in HOST_BINARIES:
        binary = bin_dir / name
        binary.write_text(f"#!/bin/sh\necho {name}\n")
        binary.chmod(0o755)
    _seed_admin_python_payload(bin_dir)


def _seed_admin_python_payload(bin_dir: Path) -> None:
    admin_pkg = bin_dir / "capsem-admin-python" / "capsem" / "admin"
    admin_pkg.mkdir(parents=True)
    (admin_pkg / "__init__.py").write_text("")
    (admin_pkg / "cli.py").write_text("def main(): return 0\n")


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
test -f "$root/usr/local/share/capsem/admin-python/capsem/admin/cli.py"
for bin in capsem capsem-service capsem-process capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray capsem-admin; do
  test -x "$root/usr/local/share/capsem/bin/$bin"
done
mkdir -p "$(dirname "$out")"
printf pkg > "$out"
"""
    )
    (tool_dir / "productbuild").write_text(
        """#!/bin/sh
set -eu
distribution=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --distribution) distribution="$2"; shift 2 ;;
    --resources|--package-path) shift 2 ;;
    *) out="$1"; shift ;;
  esac
done
test -n "$distribution"
grep 'version="1.1.0"' "$distribution" >/dev/null
if grep 'version="1.1.0\\.' "$distribution" >/dev/null; then
  echo "unexpected timestamp suffix in pkg distribution" >&2
  exit 1
fi
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
    test -f "$root/usr/share/capsem/admin-python/capsem/admin/cli.py"
    for bin in capsem capsem-service capsem-process capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray capsem-admin; do
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


def test_repack_deb_rejects_missing_assets_dir_with_explicit_output(tmp_path):
    """A missing assets dir must not be silently treated as the output .deb."""
    input_deb = tmp_path / "capsem.deb"
    input_deb.write_text("fixture\n")
    bin_dir = tmp_path / "bin"
    missing_assets_dir = tmp_path / "assets"
    output_deb = tmp_path / "out.deb"
    _seed_host_binaries(bin_dir)

    tool_dir = _fake_tool_dir(tmp_path)
    (tool_dir / "dpkg-deb").write_text(
        """#!/bin/sh
set -eu
case "$1" in
  -R)
    dest="$3"
    mkdir -p "$dest/DEBIAN"
    printf 'Package: capsem\\nVersion: 1.1.0\\nArchitecture: all\\nDescription: test\\n' > "$dest/DEBIAN/control"
    ;;
  -b)
    printf deb > "$3"
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
        [
            str(REPACK_DEB),
            str(input_deb),
            str(bin_dir),
            str(missing_assets_dir),
            str(output_deb),
        ],
        env=env,
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert result.returncode != 0, result.stdout + result.stderr
    assert "assets_dir is not a directory" in result.stderr
    assert not missing_assets_dir.exists()
    assert not output_deb.exists()


def test_prepare_admin_cli_builds_relocatable_wrapper_and_python_payload(tmp_path):
    """Release packaging must use a relocatable wrapper, never a .venv shebang."""
    tool_dir = _fake_tool_dir(tmp_path)
    (tool_dir / "uv").write_text(
        """#!/bin/sh
set -eu
if [ "$1" = "run" ] && [ "$2" = "python" ]; then
  command -v python3
  exit 0
fi
if [ "$1" != "pip" ] || [ "$2" != "install" ]; then
  echo "unexpected uv args: $*" >&2
  exit 2
fi
shift 2
target=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --python) shift 2 ;;
    --target) target="$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [ -z "$target" ]; then
  echo "missing --target" >&2
  exit 2
fi
mkdir -p "$target/capsem/admin"
printf '' > "$target/capsem/__init__.py"
printf '' > "$target/capsem/admin/__init__.py"
cat > "$target/capsem/admin/cli.py" <<'PY'
def main():
    return 0
PY
"""
    )
    (tool_dir / "python3").write_text(
        """#!/bin/sh
set -eu
if [ "${1:-}" = "-c" ]; then
  echo "3.11"
  exit 0
fi
case "$PYTHONPATH" in
  *capsem-admin-python*) exit 0 ;;
  *) echo "missing packaged python path: $PYTHONPATH" >&2; exit 3 ;;
esac
"""
    )
    for tool in ("uv", "python3"):
        (tool_dir / tool).chmod(0o755)

    out_dir = tmp_path / "release-bin"
    env = os.environ.copy()
    env["PATH"] = f"{tool_dir}:{env['PATH']}"
    result = subprocess.run(
        [str(PREPARE_ADMIN_CLI), str(out_dir)],
        env=env,
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert result.returncode == 0, result.stdout + result.stderr
    wrapper = out_dir / "capsem-admin"
    assert wrapper.exists()
    assert os.access(wrapper, os.X_OK)
    assert (out_dir / "capsem-admin-python" / "capsem" / "admin" / "cli.py").exists()
    assert (out_dir / "capsem-admin-python" / ".capsem-python-version").read_text() == "3.11\n"
    text = wrapper.read_text()
    assert ".venv" not in text
    assert ".capsem-python-version" in text
    assert "/usr/local/share/capsem/admin-python" in text
    assert "/usr/share/capsem/admin-python" in text


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
