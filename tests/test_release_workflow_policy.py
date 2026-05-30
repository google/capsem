"""Static policy tests for the GitHub release workflow."""

from __future__ import annotations

import json
import os
import re
import subprocess
from pathlib import Path


WORKFLOW = Path(__file__).parent.parent / ".github" / "workflows" / "release.yaml"
REPO_ROOT = Path(__file__).parent.parent


def _workflow_text() -> str:
    return WORKFLOW.read_text()


def _populate_manifest_python() -> str:
    text = _workflow_text()
    step = re.search(
        r"(?ms)- name: Populate and accumulate manifest\n.*?python3 <<'PY'\n(?P<code>.*?)\n          PY",
        text,
    )
    assert step, "Populate and accumulate manifest Python heredoc missing"
    return re.sub(r"(?m)^          ", "", step.group("code"))


def test_linux_release_artifacts_are_not_best_effort():
    """A release must not publish when expected Linux artifacts are missing."""
    text = _workflow_text()
    linux_job = re.search(
        r"(?ms)^  build-app-linux:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert linux_job, "build-app-linux job missing"
    assert "continue-on-error: true" not in linux_job.group("body")

    create_release = re.search(
        r"(?ms)^  create-release:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert create_release, "create-release job missing"
    body = create_release.group("body")
    assert "release-linux-arm64" in body
    assert "release-linux-x86_64" in body
    assert "continue-on-error: true" not in body
    assert "best-effort" not in body.lower()


def test_post_release_binary_e2e_is_release_blocking():
    """Post-release proof must fail when package artifacts or CLI are missing."""
    text = _workflow_text()
    verify = re.search(
        r"(?ms)^  verify-release-downloads:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert verify, "verify-release-downloads job missing"
    body = verify.group("body")
    assert "manifest.json.minisig" in body
    assert "minisign -Vm" in body
    assert "no .deb" not in body.lower()
    assert "skipping binary e2e" not in body.lower()
    assert "exit 0" not in body
    assert "PKG_PROFILES=" in body
    assert '$CAPSEM_HOME/profiles/base' in body


def test_release_packages_materialize_profiles_to_github_assets():
    """Release installers must seed profile asset URLs that exist on the release."""
    text = _workflow_text()

    assert (
        'CAPSEM_INSTALL_PROFILE_ASSET_ROOT="https://github.com/google/capsem/releases/download/v${VERSION}/{arch}-{name}"'
        in text
    )
    assert "bash scripts/build-pkg.sh" in text
    assert "bash scripts/repack-deb.sh" in text


def test_release_manifest_binary_metadata_is_preserved():
    """Adding package files must not erase generated binary metadata."""
    text = _workflow_text()
    assert "entry = new['binaries']['releases'].get(version, {})" in text
    assert "entry.update({" in text
    for field in ("date", "deprecated", "min_assets"):
        assert field in text


def test_create_release_preserves_binary_metadata(tmp_path):
    """The workflow's manifest-population script preserves compatibility fields."""
    artifacts = tmp_path / "release-artifacts"
    artifacts.mkdir()
    for name in ("capsem.pkg", "capsem-arm64.deb", "capsem-x86_64.deb"):
        (artifacts / name).write_bytes(name.encode())

    version = "1.1.123"
    manifest = {
        "format": 2,
        "assets": {
            "current": "2026.0510.10",
            "releases": {
                "2026.0510.9": {
                    "date": "2026-05-10",
                    "deprecated": False,
                    "min_binary": "1.0.0",
                    "arches": {"arm64": {"vmlinuz": {"hash": "a" * 64, "size": 1}}},
                },
                "2026.0510.10": {
                    "date": "2026-05-10",
                    "deprecated": False,
                    "min_binary": "1.0.0",
                    "arches": {"arm64": {"vmlinuz": {"hash": "b" * 64, "size": 1}}},
                },
            },
        },
        "binaries": {
            "current": version,
            "releases": {
                version: {
                    "date": "2026-05-10",
                    "deprecated": False,
                    "min_assets": "2026.0510.9",
                }
            },
        },
    }
    (artifacts / "manifest.json").write_text(json.dumps(manifest))
    env = {**os.environ, "VERSION": version, "PREV_PATH": ""}

    result = subprocess.run(
        ["python3", "-c", _populate_manifest_python()],
        cwd=tmp_path,
        env=env,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr

    updated = json.loads((artifacts / "manifest.json").read_text())
    entry = updated["binaries"]["releases"][version]
    assert entry["date"] == "2026-05-10"
    assert entry["deprecated"] is False
    assert entry["min_assets"] == "2026.0510.9"
    assert {f["name"] for f in entry["files"]} == {
        "capsem.pkg",
        "capsem-arm64.deb",
        "capsem-x86_64.deb",
    }


def test_release_provenance_covers_boot_assets_and_signed_manifest():
    """Attestation subjects must include every boot-critical release asset."""
    text = _workflow_text()
    for subject in (
        "release-artifacts/manifest.json",
        "release-artifacts/manifest.json.minisig",
        "release-artifacts/arm64/vmlinuz",
        "release-artifacts/arm64/initrd.img",
        "release-artifacts/arm64/rootfs.squashfs",
        "release-artifacts/x86_64/vmlinuz",
        "release-artifacts/x86_64/initrd.img",
        "release-artifacts/x86_64/rootfs.squashfs",
    ):
        assert subject in text


def test_release_sbom_attestation_covers_pkg_and_deb_artifacts():
    """The release SBOM must be attested against every OS package family."""
    text = _workflow_text()
    step = re.search(
        r"(?ms)- name: Attest SBOM\n(?P<body>.*?)(?=^      - name:|\Z)",
        text,
    )
    assert step, "Attest SBOM step missing"
    body = step.group("body")
    assert "actions/attest@v4" in body
    assert "predicate-type: https://spdx.dev/Document/v2.3" in body
    assert "predicate-path: release-artifacts/capsem-sbom.spdx.json" in body
    assert "release-artifacts/*.pkg" in body
    assert "release-artifacts/*.deb" in body


def test_rootfs_validation_is_hard_gated_and_canonical():
    """Release jobs must validate mounted rootfs contents from one source."""
    text = _workflow_text()
    build_assets = re.search(
        r"(?ms)^  build-assets:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert build_assets, "build-assets job missing"
    assert "scripts/validate-rootfs.sh assets/${{ matrix.arch }}/rootfs.squashfs" in build_assets.group("body")

    build_linux = re.search(
        r"(?ms)^  build-app-linux:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert build_linux, "build-app-linux job missing"
    assert "scripts/validate-rootfs.sh assets/${{ matrix.arch }}/rootfs.squashfs" in build_linux.group("body")

    create_release = re.search(
        r"(?ms)^  create-release:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert create_release, "create-release job missing"
    assert "build-assets" in create_release.group("body")
    assert "build-app-linux" in create_release.group("body")

    validator = (REPO_ROOT / "scripts" / "validate-rootfs.sh").read_text()
    assert "GUEST_BINARIES" in validator
    assert "ROOTFS_SCRIPTS" in validator
    assert "ROOTFS_SCRIPT_DIRS" in validator
    assert "ROOTFS_SUPPORT_FILES" in validator


def test_linux_deb_contents_validation_checks_each_required_payload():
    """The Linux release job must prove every package payload independently."""
    text = _workflow_text()
    verifier = (REPO_ROOT / "scripts" / "verify_deb_payload.py").read_text()
    build_linux = re.search(
        r"(?ms)^  build-app-linux:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert build_linux, "build-app-linux job missing"
    body = build_linux.group("body")
    assert "python3 scripts/verify_deb_payload.py" in body
    assert "bash scripts/prepare-admin-cli.sh target/release" in body
    assert "--minisign-pubkey config/manifest-sign.pub" in body
    assert "deb_arch=arm64" in body
    assert "deb_arch=amd64" in body
    for payload in (
        "usr/bin/capsem",
        "usr/bin/capsem-service",
        "usr/bin/capsem-process",
        "usr/bin/capsem-mcp",
        "usr/bin/capsem-mcp-aggregator",
        "usr/bin/capsem-mcp-builtin",
        "usr/bin/capsem-gateway",
        "usr/bin/capsem-tray",
        "usr/bin/capsem-tui",
        "usr/bin/capsem-admin",
        "usr/share/capsem/admin-python/capsem/admin/cli.py",
        "usr/share/capsem/assets/manifest.json",
        "usr/share/capsem/assets/manifest.json.minisig",
    ):
        assert payload in verifier


def test_release_prepares_packaged_admin_cli_before_os_packages():
    """Release packaging must include a relocatable capsem-admin payload."""
    text = _workflow_text()
    build_macos = re.search(
        r"(?ms)^  build-app-macos:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert build_macos, "build-app-macos job missing"
    mac_body = build_macos.group("body")
    assert "bash scripts/prepare-admin-cli.sh target/release" in mac_body
    assert mac_body.index("bash scripts/prepare-admin-cli.sh target/release") < mac_body.index(
        "bash scripts/build-pkg.sh"
    )

    build_linux = re.search(
        r"(?ms)^  build-app-linux:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert build_linux, "build-app-linux job missing"
    linux_body = build_linux.group("body")
    assert "bash scripts/prepare-admin-cli.sh target/release" in linux_body
    assert linux_body.index("bash scripts/prepare-admin-cli.sh target/release") < linux_body.index(
        "bash scripts/repack-deb.sh"
    )


def test_macos_pkg_signature_and_gatekeeper_are_release_blocking():
    """Release macOS packages must be signed with Installer ID and assessed."""
    text = _workflow_text()
    preflight = re.search(
        r"(?ms)^  preflight:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert preflight, "preflight job missing"
    preflight_body = preflight.group("body")
    assert "APPLE_INSTALLER_SIGNING_IDENTITY" in preflight_body
    assert "Developer ID Installer:" in preflight_body

    build_macos = re.search(
        r"(?ms)^  build-app-macos:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert build_macos, "build-app-macos job missing"
    body = build_macos.group("body")
    assert '"$APPLE_INSTALLER_SIGNING_IDENTITY"' in body
    assert "pkgutil --check-signature" in body
    assert "spctl -a -vv -t install" in body


def test_macos_pkg_payload_manifest_validation_is_single_use():
    """The pkg expansion dir must be fresh so validation is deterministic."""
    text = _workflow_text()
    build_macos = re.search(
        r"(?ms)^  build-app-macos:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert build_macos, "build-app-macos job missing"
    body = build_macos.group("body")
    assert body.count("- name: Verify .pkg payload manifest") == 1
    assert 'rm -rf "$EXPANDED"' in body


def test_linux_app_manifest_signing_installs_minisign_before_use():
    """Linux release app jobs must install minisign before signing manifests."""
    text = _workflow_text()
    build_linux = re.search(
        r"(?ms)^  build-app-linux:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert build_linux, "build-app-linux job missing"
    body = build_linux.group("body")
    install_pos = body.index("sudo apt-get install -y --no-install-recommends minisign zstd")
    sign_pos = body.index("minisign -S -s /tmp/manifest-sign.key")
    assert install_pos < sign_pos
    assert "minisign \\" not in body[body.index("Install Tauri system deps"):body.index("Build frontend")]


def test_install_e2e_downloads_built_assets_before_running_recipe():
    """Install E2E must not depend on an untracked local assets/ directory."""
    text = _workflow_text()
    test_install = re.search(
        r"(?ms)^  test-install:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)",
        text,
    )
    assert test_install, "test-install job missing"
    body = test_install.group("body")

    assert "needs: [preflight, build-assets]" in body
    assert "actions/download-artifact@v8" in body
    assert "name: vm-assets-arm64" in body
    assert "path: assets/arm64/" in body
    assert "b3sum" in body
    assert "minisign" in body


def test_preflight_compares_guest_bins_to_canonical_rootfs_list():
    """Local release preflight must compare Cargo bins with GUEST_BINARIES."""
    preflight = (REPO_ROOT / "scripts" / "preflight.sh").read_text()
    assert "capsem.builder.docker import GUEST_BINARIES" in preflight
    assert "capsem-agent [[bin]] entries match GUEST_BINARIES" in preflight
    assert "scripts/validate-rootfs.sh" in preflight


def test_unified_manifest_uses_previous_manifest_before_version_selection():
    """Same-day releases must increment from already-published asset versions."""
    text = _workflow_text()
    step = re.search(
        r"(?ms)- name: Generate unified manifest\n(?P<body>.*?)(?=\n      - name:|\n      - uses:|\Z)",
        text,
    )
    assert step, "Generate unified manifest step missing"
    body = step.group("body")
    assert "gh release download --pattern manifest.json -D /tmp/prev-manifest" in body
    assert "cp /tmp/prev-manifest/manifest.json unified-assets/manifest.json" in body
    assert body.index("cp /tmp/prev-manifest/manifest.json unified-assets/manifest.json") < body.index("generate_checksums")


def test_local_release_preflight_checks_manifest_key_and_updater_strategy():
    """Local release checks must validate the manifest key family, not Tauri keys."""
    check_workflow = (REPO_ROOT / "scripts" / "check-release-workflow.sh").read_text()
    preflight = (REPO_ROOT / "scripts" / "preflight.sh").read_text()
    combined = check_workflow + "\n" + preflight

    assert "config/manifest-sign.pub" in combined
    assert "private/manifest-sign/capsem.key" in combined
    assert "minisign -Vm" in combined
    assert "private/tauri/capsem.key" not in check_workflow
    assert "createUpdaterArtifacts" in check_workflow
    assert "tauri-plugin-updater" in check_workflow
    assert "rootfs validation uses canonical artifact lists" in check_workflow


def test_local_dev_manifest_signing_is_bootstrap_and_doctor_prereq():
    """A fresh dev machine must install minisign before local VM recipes run."""
    bootstrap = (REPO_ROOT / "bootstrap.sh").read_text()
    doctor = (REPO_ROOT / "scripts" / "doctor-common.sh").read_text()
    doctor_macos = (REPO_ROOT / "scripts" / "doctor-macos.sh").read_text()
    install_test_dockerfile = (REPO_ROOT / "docker" / "Dockerfile.install-test").read_text()
    host_builder_dockerfile = (REPO_ROOT / "docker" / "Dockerfile.host-builder").read_text()
    preflight = (REPO_ROOT / "scripts" / "preflight.sh").read_text()
    sync_dev_assets = (REPO_ROOT / "scripts" / "sync-dev-assets.sh").read_text()
    justfile = (REPO_ROOT / "justfile").read_text()

    assert "brew install minisign" in bootstrap
    assert "brew install minisign" in doctor
    assert "minisign)      echo \"brew install minisign\"" in doctor_macos
    assert 'colima_status="$(colima status 2>&1 || true)"' in doctor_macos
    assert "colima status 2>&1 | grep -qi" not in doctor_macos
    assert "section \"Manifest Signing Tools\"" in doctor
    assert "fixable minisign" in doctor
    assert "local asset manifest signature" in doctor
    assert "verify-local-manifest-signature.sh" in doctor
    assert "manifest.json.minisig" in doctor
    assert "manifest-sign.dev.pub" in (REPO_ROOT / "scripts" / "verify-local-manifest-signature.sh").read_text()
    assert "just doctor fix" in doctor
    assert "just doctor-fix" not in doctor
    assert "minisign" in re.search(r"local tools=\((?P<tools>.*?)\)", preflight, re.S).group("tools")
    assert "minisign" in install_test_dockerfile
    assert "minisign" in host_builder_dockerfile
    assert "ERROR: minisign not installed; cannot sign local asset manifest." in sync_dev_assets
    assert "scripts/sync-dev-assets.sh" in justfile
    assert "scripts/verify-local-manifest-signature.sh" in justfile


def test_local_cross_compile_validates_one_fresh_deb_artifact():
    """Cached Docker target volumes must not let stale debs poison validation."""
    justfile = (REPO_ROOT / "justfile").read_text()
    cross_compile = re.search(
        r'(?ms)^cross-compile arch="":.*?(?=^# Generate settings-schema\.json)',
        justfile,
    )
    assert cross_compile, "cross-compile recipe missing"
    body = cross_compile.group(0)
    assert "cp -r \"assets/$TARGET_ARCH\" assets/current" in body
    assert 'b3sum "$arch_name/vmlinuz" "$arch_name/initrd.img" "$arch_name/rootfs.squashfs" >> B3SUMS' in body
    assert "python3 scripts/gen_manifest.py assets Cargo.toml" in body
    assert "bash scripts/sync-dev-assets.sh assets assets" in body
    assert "bash scripts/verify-local-manifest-signature.sh assets config/manifest-sign.pub" in body
    assert 'VSOCK_FLAG="--device /dev/vhost-vsock"' in body
    assert 'VSOCK_SECURITY_FLAG="--security-opt seccomp=unconfined"' in body
    assert "$VSOCK_FLAG" in body
    assert "$VSOCK_SECURITY_FLAG" in body
    assert 'capsem-frontend-node-modules-$TARGET_ARCH:/src/frontend/node_modules' in body
    assert 'DEB_DIR=/cargo-target/\\$RUST_TARGET/release/bundle/deb' in body
    assert 'rm -f \\"\\$DEB_DIR\\"/*.deb' in body
    assert 'DEBS=(\\"\\$DEB_DIR\\"/*.deb)' in body
    assert 'expected exactly one deb artifact' in body
    assert "cargo build --release --target \\$RUST_TARGET {{host_crates}}" in body
    assert "UV_PROJECT_ENVIRONMENT=/cargo-target/capsem-package-venv bash scripts/prepare-admin-cli.sh /cargo-target/\\$RUST_TARGET/release" in body
    assert 'bash scripts/repack-deb.sh \\"\\$DEB\\" /cargo-target/\\$RUST_TARGET/release assets \\"\\$DEB\\"' in body
    assert 'UV_PROJECT_ENVIRONMENT=/cargo-target/capsem-package-venv uv run python scripts/verify_deb_payload.py \\"\\$DEB\\" --version \\"\\$PACKAGE_VERSION\\" --architecture \\"\\$DPKG_ARCH\\" --minisign-pubkey assets/manifest-sign.dev.pub' in body
    assert 'dpkg-deb --info \\"\\$DEB\\"' in body
    assert 'rm -f /src/dist/Capsem_*_\\"\\$DPKG_ARCH\\".deb' in body
    assert 'dpkg-deb --info /cargo-target/\\$RUST_TARGET/release/bundle/deb/*.deb' not in body
    assert 'dpkg --unpack \\"\\$DEB\\"' in body
    assert '--binary /usr/bin/capsem' in body


def test_check_assets_requires_profile_image_inventory():
    """Artifact-gated tests need image inventory, not only bootable files."""
    justfile = (REPO_ROOT / "justfile").read_text()
    check_assets = re.search(
        r"(?ms)^_check-assets:\n(?P<body>.*?)(?=^_pnpm-install:)",
        justfile,
    )
    assert check_assets, "_check-assets recipe missing"
    body = check_assets.group("body")

    assert "vmlinuz initrd.img rootfs.squashfs image-inventory.json" in body
    assert 'missing+=("$arch/$f")' in body
    assert 'just build-assets "$arch"' in body
    assert "just build-assets\n" not in body


def _just_install_body() -> str:
    justfile = (REPO_ROOT / "justfile").read_text()
    install = re.search(
        r"(?ms)^install:.*?(?=^# Run install e2e tests in Docker)",
        justfile,
    )
    assert install, "install recipe missing"
    return install.group(0)


def test_local_install_removes_old_runtime_before_installing_package():
    """`just install` must prove the old runtime is gone before reinstalling."""
    body = _just_install_body()

    assert "install: _pnpm-install _stamp-version _check-assets\n" in body
    assert "install: _pnpm-install _stamp-version _check-assets _pack-initrd" not in body
    assert 'echo "=== Rebuilding profile-derived VM assets ==="' in body
    assert 'just build-assets "$HOST_ARCH" "{{default_asset_profile}}"' in body
    assert 'echo "=== Repacking VM assets ==="' in body
    assert "\n    just _pack-initrd\n" in body
    assert 'echo "=== Keeping existing local profile metadata coherent ==="' in body
    assert "scripts/materialize-install-profiles.py" in body
    assert 'INSTALL_ASSETS_DIR="$ROOT/.capsem-assets/install"' in body
    assert 'bash scripts/sync-dev-assets.sh "{{assets_dir}}" "$INSTALL_ASSETS_DIR"' in body
    assert 'echo "=== Clean uninstalling existing local Capsem ==="' in body
    assert 'CAPSEM_SETTINGS_BACKUP="$(mktemp -d' in body
    assert 'preserve_setting "service.toml"' in body
    assert 'restore_setting "service.toml"' in body
    assert 'preserve_setting "profiles"' not in body
    assert 'restore_setting "profiles"' not in body
    assert '"$HOME/.capsem/bin/capsem" uninstall --yes' in body
    assert '"$ROOT/target/release/capsem" uninstall --yes' in body
    assert "assert_clean_uninstall" in body
    assert 'LaunchAgent still exists after uninstall' in body
    assert 'runtime bin dir still exists after uninstall' in body
    assert 'runtime run-state still exists after uninstall' in body
    assert '! -name persistent' in body
    assert '! -name persistent_registry.json' in body

    clean_pos = body.index("=== Clean uninstalling existing local Capsem ===")
    snapshot_pos = body.index("=== Snapshotting package asset payload ===")
    assert_pos = body.index("\n    assert_clean_uninstall", clean_pos)
    rebuild_pos = body.index("=== Rebuilding profile-derived VM assets ===")
    repack_pos = body.index("=== Repacking VM assets ===")
    repair_pos = body.index("=== Keeping existing local profile metadata coherent ===")
    mac_install_pos = body.index('sudo installer -pkg "$PKG" -target /')
    linux_install_pos = body.index('sudo apt install -y "$DEB"')
    assert rebuild_pos < repack_pos < repair_pos < snapshot_pos < clean_pos < assert_pos < mac_install_pos
    assert rebuild_pos < repack_pos < repair_pos < snapshot_pos < clean_pos < assert_pos < linux_install_pos


def test_local_install_reruns_setup_after_restoring_settings():
    """`just install` must not undo package postinstall setup with restored state."""
    body = _just_install_body()

    assert 'preserve_setting "profiles"' not in body
    assert 'restore_setting "profiles"' not in body
    assert 'echo "=== Restoring preserved settings ==="' in body
    assert 'echo "=== Syncing locally built assets into ~/.capsem/assets ==="' in body
    assert 'echo "=== Finalizing installed setup ==="' in body
    assert '"$HOME/.capsem/bin/capsem" setup --non-interactive --accept-detected' in body

    restore_pos = body.index('echo "=== Restoring preserved settings ==="')
    sync_pos = body.index('echo "=== Syncing locally built assets into ~/.capsem/assets ==="')
    setup_pos = body.index('echo "=== Finalizing installed setup ==="')
    restart_pos = body.index('echo "=== Restarting installed service ==="')
    assert restore_pos < sync_pos < setup_pos < restart_pos


def test_local_install_uses_same_native_install_commands_as_install_sh():
    """The local installer path should match what users run from install.sh."""
    body = _just_install_body()
    install_sh = (REPO_ROOT / "site" / "public" / "install.sh").read_text()

    assert 'sudo installer -pkg "$PKG_PATH" -target /' in install_sh
    assert 'sudo installer -pkg "$PKG" -target /' in body
    assert 'open -W "$PKG"' not in body

    assert 'sudo apt install -y "$DEB_PATH"' in install_sh
    assert 'sudo apt install -y "$DEB"' in body
    assert "sudo dpkg -i" not in body


def test_local_install_removes_stale_tauri_bundle_before_rebuild():
    """Root-owned or stale app bundles must not break the repeatable install loop."""
    body = _just_install_body()

    assert 'remove_stale_path "target/release/bundle/macos/Capsem.app"' in body
    assert 'sudo rm -rf "$path"' in body
    assert body.index('remove_stale_path "target/release/bundle/macos/Capsem.app"') < body.index(
        "cargo tauri build --bundles app"
    )


def test_local_install_verifies_fresh_install_and_guest_network():
    """Service-only health is insufficient; the installed VM must resolve and curl."""
    body = _just_install_body()

    for binary in (
        "capsem",
        "capsem-service",
        "capsem-process",
        "capsem-mcp",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-gateway",
        "capsem-tray",
        "capsem-tui",
    ):
        assert f'assert_executable "$HOME/.capsem/bin/{binary}"' in body

    assert "WARNING: Service not responding" not in body
    assert 'BUILT_VERSION=$("$ROOT/target/release/capsem" version)' in body
    assert 'INSTALLED_VERSION=$("$HOME/.capsem/bin/capsem" version)' in body
    assert 'curl -fsS --unix-socket "$HOME/.capsem/run/service.sock"' in body
    assert 'curl -fsS "http://127.0.0.1:$GATEWAY_PORT/health"' in body
    assert "python3 scripts/capture-install-status.py \\" in body
    assert '--capsem-bin "$HOME/.capsem/bin/capsem"' in body
    assert "--label just-install" in body
    assert '"$HOME/.capsem/bin/capsem" run' in body
    assert "getent hosts elie.net" in body
    assert "getent hosts localhost" in body
    assert "getent hosts generativelanguage.googleapis.com" in body
    assert "curl -fsS --connect-timeout 10 https://elie.net" in body
    assert "agy --version" in body

    gateway_pos = body.index("=== Verifying gateway health ===")
    status_pos = body.index("=== Capturing installed status ===")
    guest_pos = body.index("=== Verifying guest DNS and HTTPS ===")
    assert gateway_pos < status_pos < guest_pos


def test_install_e2e_prepares_clean_checkout_assets_before_repack():
    """Release install E2E starts from a clean checkout, so assets must be materialized."""
    justfile = (REPO_ROOT / "justfile").read_text()
    test_install = re.search(
        r"(?ms)^test-install:\n(?P<body>.*?)(?=^# Wait for CI to build)",
        justfile,
    )
    assert test_install, "test-install recipe missing"
    body = test_install.group("body")

    assert 'ASSETS_HOST="$(python3 -c' in body
    assert 'WORKDIR_CONTAINER="/work/src"' in body
    assert 'ASSETS_CONTAINER="$WORKDIR_CONTAINER/{{assets_dir}}"' in body
    assert '-v "$PWD":/checkout:ro' in body
    assert '-v "$ASSETS_HOST":/asset-source:ro' in body
    assert "tar -C '$WORKDIR_CONTAINER' -xf -" in body
    assert "tar -C '$ASSETS_CONTAINER' -xf -" in body
    assert "chown -R capsem:capsem /work" in body
    assert "SRC_UID=$(docker exec \"$CONTAINER\" stat -c %u /src)" not in body
    assert "usermod -o -u" not in body
    assert "chown -R capsem:capsem /src" not in body
    assert "UV_PROJECT_ENVIRONMENT=/cargo-target/install-test-venv" in body
    assert "large local uid/gid values in .deb ar" in body
    assert "cd '$WORKDIR_CONTAINER'" in body
    assert "cd /work/src" in body
    assert 'bash scripts/prepare-install-assets.sh "$ASSETS_CONTAINER" Cargo.toml "${INSTALL_ARCH:-$(uname -m)}"' in body
    assert 'bash scripts/repack-deb.sh "$DEB" /cargo-target/debug "$ASSETS_CONTAINER" "$DEB"' in body
    assert '-e CAPSEM_ASSETS_SRC="$ASSETS_CONTAINER"' in body
    assert "uv run --group dev python -m pytest tests/capsem-install/" in body
    assert 'scripts/repack-deb.sh "$DEB" /cargo-target/debug assets' not in body

    prep_script = (REPO_ROOT / "scripts" / "prepare-install-assets.sh").read_text()
    assert "Build assets on the host first: just build-assets $INSTALL_ARCH" in prep_script
    assert 'just build-assets "$INSTALL_ARCH"' not in prep_script
    assert 'b3sum "$INSTALL_ARCH/vmlinuz" "$INSTALL_ARCH/initrd.img" "$INSTALL_ARCH/rootfs.squashfs" > B3SUMS' in prep_script
    assert 'python3 scripts/gen_manifest.py "$ASSETS_DIR" "$CARGO_TOML"' in prep_script
    assert 'python3 scripts/create_hash_assets.py "$ASSETS_DIR"' in prep_script
    assert 'bash scripts/sync-dev-assets.sh "$ASSETS_DIR" "$ASSETS_DIR"' in prep_script
    assert 'bash scripts/verify-local-manifest-signature.sh "$ASSETS_DIR" config/manifest-sign.pub' in prep_script

    sync_script = (REPO_ROOT / "scripts" / "sync-dev-assets.sh").read_text()
    assert '[[ -f "$src_file" ]] || continue' in sync_script


def test_simulate_install_copies_only_arch_asset_files(tmp_path):
    """Nested source asset dirs must not poison install fixture refresh."""
    bin_src = tmp_path / "bin"
    assets_src = tmp_path / "assets"
    capsem_home = tmp_path / "home" / ".capsem"
    bin_src.mkdir()

    binaries = (
        "capsem",
        "capsem-service",
        "capsem-process",
        "capsem-mcp",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-gateway",
        "capsem-tray",
        "capsem-tui",
    )
    for binary in binaries:
        path = bin_src / binary
        path.write_text("#!/bin/sh\n[ \"$1\" = version ] && echo 'capsem 1.1.0 build test'\n")
        path.chmod(0o755)

    machine = os.uname().machine.lower()
    arch = "arm64" if machine in {"arm64", "aarch64"} else "x86_64"
    arch_src = assets_src / arch
    arch_src.mkdir(parents=True)
    for name in ("vmlinuz", "initrd.img", "rootfs.squashfs"):
        (arch_src / name).write_text(name)
    (arch_src / arch).mkdir()
    (assets_src / "manifest.json").write_text(
        json.dumps(
            {
                "format": 2,
                "assets": {
                    "current": "2026.0524.1",
                    "releases": {
                        "2026.0524.1": {
                            "date": "2026-05-24",
                            "deprecated": False,
                            "min_binary": "1.0.0",
                            "arches": {
                                arch: {
                                    "vmlinuz": {"hash": "1" * 64, "size": len("vmlinuz")},
                                    "initrd.img": {
                                        "hash": "2" * 64,
                                        "size": len("initrd.img"),
                                    },
                                    "rootfs.squashfs": {
                                        "hash": "3" * 64,
                                        "size": len("rootfs.squashfs"),
                                    },
                                }
                            },
                        }
                    },
                },
            }
        )
    )
    (assets_src / "manifest.json.minisig").write_text("sig")
    (assets_src / "manifest-sign.dev.pub").write_text("pub")

    result = subprocess.run(
        [
            "bash",
            str(REPO_ROOT / "scripts" / "simulate-install.sh"),
            str(bin_src),
            str(assets_src),
        ],
        cwd=REPO_ROOT,
        env={**os.environ, "CAPSEM_HOME": str(capsem_home)},
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert result.returncode == 0, result.stderr
    assert (capsem_home / "assets" / arch / "vmlinuz").is_file()
    assert not (capsem_home / "assets" / arch / arch).exists()


def test_cut_release_prepares_local_tag_without_pushing():
    """Release push/tag publication is a deliberate manual step."""
    justfile = (REPO_ROOT / "justfile").read_text()
    cut_release = re.search(
        r"(?ms)^cut-release: test _stamp-version\n(?P<body>.*?)(?=^# Check dev tools)",
        justfile,
    )
    assert cut_release, "cut-release recipe missing"
    body = cut_release.group("body")

    assert 'git tag "$TAG"' in body
    assert "uv.lock" in body
    assert 'git push origin main "$TAG"' not in body
    assert 'git push origin HEAD:main' in body
    assert 'git push origin $TAG' in body
    assert 'just release $TAG' in body
