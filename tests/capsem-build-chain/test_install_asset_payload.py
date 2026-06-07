"""Install package asset-payload contract tests."""

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_just_install_does_not_sync_assets_after_installer() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    install_body = justfile.split("\n# Run install e2e tests", 1)[0]

    assert "Syncing local dev assets" not in install_body
    assert "scripts/sync-dev-assets.sh" not in install_body
    assert "CAPSEM_PKG_ASSET_MODE=current-arch bash scripts/build-pkg.sh" in install_body
    assert "CAPSEM_DEB_ASSET_MODE=current-arch bash scripts/repack-deb.sh" in install_body


def test_package_builders_support_current_arch_asset_payloads() -> None:
    build_pkg = (PROJECT_ROOT / "scripts" / "build-pkg.sh").read_text()
    repack_deb = (PROJECT_ROOT / "scripts" / "repack-deb.sh").read_text()
    deb_postinst = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()

    assert "CAPSEM_PKG_ASSET_MODE" in build_pkg
    assert 'current-arch)' in build_pkg
    assert 'bash "$SCRIPT_DIR/sync-dev-assets.sh" "$ASSETS_DIR" "$SHARE_DIR/assets"' in build_pkg

    assert "CAPSEM_DEB_ASSET_MODE" in repack_deb
    assert 'bash "$SCRIPT_DIR/sync-dev-assets.sh" "$ASSETS_DIR"' in repack_deb
    assert "/usr/share/capsem/assets" in deb_postinst


def test_macos_postinstall_adds_capsem_bin_to_fish_path() -> None:
    postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()

    assert ".config/fish/config.fish" in postinstall
    assert "fish_add_path" in postinstall
    assert "grep -qF 'fish_add_path --path \"$HOME/.capsem/bin\"'" in postinstall


def test_security_event_rows_go_through_security_engine_emitter() -> None:
    roots = [
        PROJECT_ROOT / "crates" / "capsem-core" / "src",
        PROJECT_ROOT / "crates" / "capsem-process" / "src",
    ]
    allowed_files = {
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "security_engine" / "mod.rs",
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "security_engine" / "tests.rs",
    }
    patterns = [
        "write(WriteOp::",
        "write(capsem_logger::WriteOp::",
        "try_write(WriteOp::",
        "try_write(capsem_logger::WriteOp::",
        "try_emit_security_write(",
    ]

    violations: list[str] = []
    for root in roots:
        for path in root.rglob("*.rs"):
            if path in allowed_files or "/tests/" in path.as_posix():
                continue
            text = path.read_text()
            for lineno, line in enumerate(text.splitlines(), start=1):
                if any(pattern in line for pattern in patterns):
                    rel = path.relative_to(PROJECT_ROOT)
                    violations.append(f"{rel}:{lineno}: {line.strip()}")

    assert not violations, (
        "security/logging rows must be emitted through "
        "capsem_core::security_engine::{emit_security_write,emit_security_write_blocking}; "
        "direct DbWriter WriteOp sends found:\n" + "\n".join(violations)
    )
