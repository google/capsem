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
        "usr/share/capsem/assets/manifest.json",
        "usr/share/capsem/assets/manifest.json.minisig",
    ):
        assert payload in verifier


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


def test_policy_hook_openapi_artifact_is_tracked_and_valid():
    """Clean checkouts must include the checked-in Policy Hook OpenAPI spec."""
    artifact = REPO_ROOT / "config" / "policy-hook-openapi.json"
    result = subprocess.run(
        ["git", "ls-files", "--error-unmatch", "config/policy-hook-openapi.json"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "config/policy-hook-openapi.json"
    with artifact.open() as f:
        parsed = json.load(f)
    assert parsed["openapi"].startswith("3.")
    assert "/v1/policy/decision" in parsed["paths"]


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
    assert 'DEB_DIR=/cargo-target/\\$RUST_TARGET/release/bundle/deb' in body
    assert 'rm -f \\"\\$DEB_DIR\\"/*.deb' in body
    assert 'DEBS=(\\"\\$DEB_DIR\\"/*.deb)' in body
    assert 'expected exactly one deb artifact' in body
    assert 'dpkg-deb --info \\"\\$DEB\\"' in body
    assert 'rm -f /src/dist/Capsem_*_\\"\\$DPKG_ARCH\\".deb' in body
    assert 'dpkg-deb --info /cargo-target/\\$RUST_TARGET/release/bundle/deb/*.deb' not in body
    assert 'dpkg -i \\"\\$DEB\\"' in body


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

    assert 'echo "=== Clean uninstalling existing local Capsem ==="' in body
    assert 'CAPSEM_SETTINGS_BACKUP="$(mktemp -d' in body
    for setting in ("user.toml", "corp.toml", "corp-source.json"):
        assert setting in body
    assert '"$HOME/.capsem/bin/capsem" uninstall --yes' in body
    assert '"$ROOT/target/release/capsem" uninstall --yes' in body
    assert "assert_clean_uninstall" in body
    assert 'LaunchAgent still exists after uninstall' in body
    assert 'runtime bin dir still exists after uninstall' in body
    assert 'runtime run-state still exists after uninstall' in body
    assert '! -name persistent' in body
    assert '! -name persistent_registry.json' in body

    clean_pos = body.index("=== Clean uninstalling existing local Capsem ===")
    assert_pos = body.index("\n    assert_clean_uninstall", clean_pos)
    mac_install_pos = body.index('sudo installer -pkg "$PKG" -target /')
    linux_install_pos = body.index('sudo apt install -y "$DEB"')
    assert clean_pos < assert_pos < mac_install_pos
    assert clean_pos < assert_pos < linux_install_pos


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
    assert "getent hosts generativelanguage.googleapis.com" in body
    assert "curl -fsS --connect-timeout 10 https://elie.net" in body

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

    assert 'bash scripts/prepare-install-assets.sh "{{assets_dir}}" Cargo.toml "${INSTALL_ARCH:-$(uname -m)}"' in body
    assert 'bash scripts/repack-deb.sh "$DEB" /cargo-target/debug /src/{{assets_dir}} "$DEB"' in body
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
