"""Install package asset-payload contract tests."""

import importlib.util
from types import ModuleType
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _just_recipe_block(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith(name))
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def _workflow_job_blocks(workflow: str) -> dict[str, str]:
    lines = workflow.splitlines()
    starts: list[tuple[str, int]] = []
    for index, line in enumerate(lines):
        if line.startswith("  ") and not line.startswith("    ") and line.rstrip().endswith(":"):
            starts.append((line.strip()[:-1], index))

    blocks: dict[str, str] = {}
    for offset, (name, start) in enumerate(starts):
        end = starts[offset + 1][1] if offset + 1 < len(starts) else len(lines)
        blocks[name] = "\n".join(lines[start:end])
    return blocks


def _load_local_release_glowup() -> ModuleType:
    path = PROJECT_ROOT / "scripts" / "local-release-glowup.py"
    spec = importlib.util.spec_from_file_location("local_release_glowup", path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_just_install_does_not_sync_assets_after_installer() -> None:
    install_body = _just_recipe_block("install:")

    assert "Syncing local dev assets" not in install_body
    assert "scripts/sync-dev-assets.sh" not in install_body
    assert "CAPSEM_PKG_ASSET_MODE=current-arch bash scripts/build-pkg.sh" not in install_body
    assert "CAPSEM_DEB_ASSET_MODE=current-arch bash scripts/repack-deb.sh" not in install_body
    assert "bash scripts/build-pkg.sh" in install_body
    assert "bash scripts/repack-deb.sh --manifest" in install_body
    assert (
        'MANIFEST_URL="${CAPSEM_INSTALL_MANIFEST_URL:-https://release.capsem.org/assets/stable/manifest.json}"'
        in install_body
    )
    assert '--manifest "$MANIFEST_URL"' in install_body
    assert "file://$PWD/{{assets_dir}}/manifest.json" not in install_body
    assert '"target/config"' in install_body
    assert (
        "install: _pnpm-install _stamp-version _check-assets _pack-initrd _materialize-config"
        in install_body
    )
    assert "pkill -9 -x capsem-app" in install_body


def test_just_install_invokes_package_without_gui_installer_block() -> None:
    install_body = _just_recipe_block("install:")

    assert 'PKG="packages/Capsem-$VERSION.pkg"' in install_body
    assert 'open -W "$PKG"' not in install_body
    assert 'installer -pkg "$PKG"' in install_body
    assert '"$HOME/.capsem/bin/capsem" status' in install_body
    assert '"$HOME/.capsem/bin/capsem" debug' in install_body


def test_cross_compile_repacks_deb_before_exact_systemd_install_proof() -> None:
    block = _just_recipe_block("cross-compile")

    companion_pos = block.find("--- Build companion host binaries ---")
    tauri_pos = block.find("cargo tauri build --target")
    repack_pos = block.find("scripts/repack-deb.sh")
    validate_pos = block.find("dpkg-deb --contents")
    copy_pos = block.find('cp \\"\\$DEB\\" /src/dist/')
    proof_pos = block.find("just _prove-linux-deb")

    assert companion_pos != -1
    assert tauri_pos != -1
    assert repack_pos != -1
    assert validate_pos != -1
    assert copy_pos != -1
    assert proof_pos != -1
    assert (
        companion_pos
        < tauri_pos
        < repack_pos
        < validate_pos
        < copy_pos
        < proof_pos
    )
    assert 'dpkg -i \\"\\$DEB\\"' not in block
    assert "CAPSEM_REQUIRE_LINUX_DEB_PROOF" in block
    assert "exact Debian package proof requires native Linux KVM" in block
    assert (
        'MANIFEST_URL="${CAPSEM_INSTALL_MANIFEST_URL:-https://release.capsem.org/assets/stable/manifest.json}"'
        in block
    )
    assert 'MANIFEST_CHANNEL="${CAPSEM_INSTALL_CHANNEL:-stable}"' in block
    assert '-e "CAPSEM_INSTALL_MANIFEST_URL=$MANIFEST_URL"' in block
    assert 'scripts/repack-deb.sh --manifest \\"\\$CAPSEM_INSTALL_MANIFEST_URL\\"' in block
    assert 'file://\\$PWD/assets/manifest.json' not in block
    assert 'CAPSEM_PROOF_MANIFEST_URL="$MANIFEST_URL"' in block
    assert 'CAPSEM_PROOF_MANIFEST_CHANNEL="$MANIFEST_CHANNEL"' in block
    assert 'CAPSEM_PROOF_DEB="$DEB"' in block
    assert "capsem-admin)\\$'" in block
    assert '-e "HOST_UID=$HOST_UID"' in block
    assert '-e "HOST_GID=$HOST_GID"' in block
    assert 'trap \'chown -R \\"\\$HOST_UID:\\$HOST_GID\\"' in block
    assert "/src/frontend/node_modules /src/frontend/dist" in block
    assert "dpkg -i /cargo-target/$RUST_TARGET/release/bundle/deb/*.deb" not in block


def test_exact_linux_deb_proof_uses_systemd_and_proves_guest_shell() -> None:
    block = _just_recipe_block("_prove-linux-deb")

    assert "capsem-install-test" in block
    assert "/usr/lib/systemd/systemd" in block
    assert "--privileged --cgroupns=host" in block
    assert "--security-opt seccomp=unconfined" in block
    assert "--device /dev/kvm" in block
    assert "--device /dev/vhost-vsock" in block
    assert '-v "$ROOT:/src:ro"' in block
    assert 'dpkg -i "$CONTAINER_DEB"' in block
    assert "apt-get install -f -y" in block
    assert "dpkg-query -W" in block
    for binary in (
        "capsem",
        "capsem-admin",
        "capsem-app",
        "capsem-gateway",
        "capsem-mcp",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-process",
        "capsem-service",
        "capsem-tray",
        "capsem-tui",
    ):
        assert binary in block
    assert 'test -x "/usr/bin/$bin"' in block
    assert '"/usr/bin/$bin" --version | grep -F "$EXPECTED_VERSION"' in block
    assert 'grep -F "Installed: true"' in block
    assert 'grep -F "Running:   true"' in block
    assert 'grep -F "Service:   ok"' in block
    assert 'grep -F "Gateway:   ok"' in block
    assert "Profiles:" in block
    assert "scripts/prove-installed-shell.py" in block
    assert "CAPSEM_QUALIFIED_DEB_SHELL_OK" in block
    assert "scripts/verify-installed-release.py" in block
    assert 'MANIFEST_URL="${CAPSEM_PROOF_MANIFEST_URL:?exact package proof requires' in block
    assert 'MANIFEST_CHANNEL="${CAPSEM_PROOF_MANIFEST_CHANNEL:?exact package proof requires' in block
    assert 'DEB_INPUT="${CAPSEM_PROOF_DEB:?exact package proof requires' in block
    assert "{{deb}}" not in block
    assert '--manifest-url "$MANIFEST_URL"' in block
    assert '--channel "$MANIFEST_CHANNEL"' in block
    assert '--package-version "$EXPECTED_VERSION"' in block
    assert "trap cleanup EXIT" in block
    assert "dpkg -i \"$CONTAINER_DEB\" 2>/dev/null || true" not in block


def test_release_qualification_requires_exact_linux_deb_proof() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release-qualification.yaml").read_text()

    assert 'CAPSEM_REQUIRE_LINUX_DEB_PROOF: "1"' in workflow
    assert (
        "CAPSEM_INSTALL_MANIFEST_URL: https://release.capsem.org/assets/stable/manifest.json"
        in workflow
    )
    assert "CAPSEM_INSTALL_CHANNEL: stable" in workflow


def test_install_test_restores_host_workspace_ownership() -> None:
    block = _just_recipe_block("test-install")

    assert "HOST_UID=$(id -u)" in block
    assert "HOST_GID=$(id -g)" in block
    assert "chown -R $HOST_UID:$HOST_GID /src" in block
    assert "trap cleanup EXIT" in block
    assert 'docker rm -f "$CONTAINER"' in block


def test_install_test_removes_stale_container_before_fail_closed_cache_reset() -> None:
    block = _just_recipe_block("test-install")

    remove_stale = block.index('docker rm -f "$CONTAINER"')
    inspect_cache = block.index('VOLUME_LINE=$(docker system df -v')
    reset_cache = block.index('docker volume rm capsem-install-target')

    assert remove_stale < inspect_cache < reset_cache
    assert 'docker volume rm capsem-install-target >/dev/null 2>&1 || true' not in block
    assert 'Failed to reset oversized capsem-install-target volume' in block
    assert 'docker ps -a --filter volume=capsem-install-target' in block


def test_install_test_runs_local_release_glowup_from_real_package() -> None:
    block = _just_recipe_block("test-install")

    assert "Running local release glow-up" in block
    assert "scripts/local-release-glowup.py" in block
    assert '--input-deb "$DEB"' in block
    assert "--bin-dir /cargo-target/debug" in block
    assert "--assets-dir assets" in block
    assert "--config-root target/config" in block
    assert "just test-install" in _just_recipe_block("test:")


def test_local_release_glowup_uses_real_release_pipeline_not_fake_manifest() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "scripts/repack-deb.sh" in script
    assert "scripts/generate-host-binary-sbom.py" in script
    assert "record-binary" in script
    assert "assets" in script and "channel" in script and "build" in script
    assert "json.dumps({" not in script or "capsem.local_release_glowup.v1" in script
    assert "stable-assets-manifest.json" in script
    assert "nightly-assets-manifest.json" in script
    assert 'shutil.copy2(args.assets_dir / "manifest.json"' in script
    assert "CAPSEM_RELEASE_URL" in script
    assert "CAPSEM_RELEASE_CHANNELS_URL=" in script
    assert "update --assets --channel nightly" in script
    assert "update --assets --channel stable" in script
    assert "update --yes --channel nightly" not in script
    assert "update --yes --channel stable" not in script
    transition_gate = (
        PROJECT_ROOT / "scripts" / "check-public-binary-release.py"
    ).read_text()
    assert "run_docker_binary_transition_smoke" in transition_gate
    assert "update --yes --channel nightly" in transition_gate
    assert "update --yes --channel stable" in transition_gate
    assert "SimpleHTTPRequestHandler" in script
    assert "--network=host" not in script


def test_local_release_glowup_has_zstd_extraction_support_in_install_image() -> None:
    dockerfile = (PROJECT_ROOT / "docker" / "Dockerfile.install-test").read_text()

    assert "zstd" in dockerfile


def test_install_recipe_invokes_pytest_as_a_module_inside_container() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    recipe = justfile.split("test-install:", maxsplit=1)[1].split(
        "\n# Dispatch one serialized release workflow", maxsplit=1
    )[0]

    # /src is bind-mounted and may contain a host .venv whose console-script
    # shebang cannot exist in the Linux container. Launch via Python so uv's
    # selected interpreter owns module resolution instead.
    assert "UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test" in recipe
    assert "uv run python -m pytest tests/capsem-install/" in recipe
    assert "uv run pytest tests/capsem-install/" not in recipe


def test_full_gate_preflights_clean_install_harness_before_expensive_stages() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    full_gate = justfile.split("test: _bootstrap _install-tools", maxsplit=1)[1].split(
        "\n# Build the capsem-host-builder", maxsplit=1
    )[0]
    preflight = justfile.split("_test-install-harness-preflight:", maxsplit=1)[1].split(
        "\ntest-install:", maxsplit=1
    )[0]

    assert "just _test-install-harness-preflight" in full_gate
    assert full_gate.index("just _test-install-harness-preflight") < full_gate.index(
        "cargo clippy --workspace --all-targets"
    )
    assert "docker/Dockerfile.install-test" in preflight
    assert "UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test" in preflight
    assert "uv run python -m pytest --version" in preflight
    assert "sudo -n true" in preflight
    assert "docker build --no-cache" in preflight


def test_standalone_install_gate_preflights_privileged_helper() -> None:
    block = _just_recipe_block("test-install")

    preflight = block.index("just _test-install-harness-preflight")
    start_container = block.index('echo "Starting systemd container..."')

    assert preflight < start_container


def test_binary_release_sbom_jobs_install_zstd_for_deb_payloads() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()

    for job_name in ("create-release", "assemble-release-channel"):
        job = _workflow_job_blocks(workflow)[job_name]
        assert "Install host SBOM archive deps" in job
        assert "zstd" in job
        assert job.index("Install host SBOM archive deps") < job.index(
            "Generate packaged host SBOM"
        )


def test_local_release_glowup_channel_build_uses_local_release_urls() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    build_channel = script.split("def build_channel(", maxsplit=1)[1].split(
        "\ndef copy_artifact_tree", maxsplit=1
    )[0]

    assert "CAPSEM_RELEASE_URL" in build_channel
    assert 'f"{base_url}/releases/download/{channel}"' in build_channel
    assert "--asset-source-base" in build_channel
    assert 'f"{base_url}/assets/releases/{{asset_version}}"' in build_channel
    assert "stage_vm_asset_blobs(stable_manifest, args.assets_dir, dist)" in script


def test_local_release_glowup_rejects_root_relative_runtime_asset_urls() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    checker = script.split("def check_generated_release(", maxsplit=1)[1].split(
        "\ndef release_asset_urls", maxsplit=1
    )[0]

    assert 'elif url.startswith("/")' not in checker
    assert "generated VM asset URL is not absolute" in checker


def test_local_release_glowup_validates_vm_asset_blobs_are_served() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "release_asset_urls" in script
    assert "release is missing VM asset blob" in script
    assert '"/assets/releases/"' in script


def test_local_release_glowup_preflights_stable_and_nightly_manifests() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert (
        'check_generated_release(base_url, stable_manifest_url, stable_deb, dist, "stable")'
        in script
    )
    assert (
        'check_generated_release(base_url, nightly_manifest_url, nightly_deb, dist, "nightly")'
        in script
    )


def test_local_release_glowup_generated_release_checker_rejects_missing_asset_blob(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    dist = tmp_path / "dist"
    dist.mkdir()
    deb = tmp_path / "Capsem_1.5.1_amd64.deb"

    with glowup.local_release_server(dist) as base_url:
        package_path = dist / "releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
        package_path.parent.mkdir(parents=True)
        package_path.write_bytes(b"fixture deb")
        manifest_path = dist / "assets" / "stable" / "manifest.json"
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(
            """{
  "packages": [
    {
      "name": "Capsem_1.5.1_amd64.deb",
      "url": "%s/releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
    }
  ],
  "profiles": {
    "co-work": {
      "architectures": [
        {
          "images": [
            {"url": "%s/assets/releases/2026.0709.13/x86_64-rootfs.erofs"}
          ],
          "evidence": [
            {"url": "%s/assets/releases/2026.0709.13/obom.cdx.json"}
          ]
        }
      ]
    }
  }
}
"""
            % (base_url, base_url, base_url),
            encoding="utf-8",
        )

        try:
            glowup.check_generated_release(
                base_url,
                f"{base_url}/assets/stable/manifest.json",
                deb,
                dist,
                "stable",
            )
        except SystemExit as error:
            assert "generated stable release is missing VM asset blob" in str(error)
            assert "x86_64-rootfs.erofs" in str(error)
        else:
            raise AssertionError("missing VM asset blob was accepted")


def test_local_release_glowup_generated_release_checker_accepts_local_assets(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    dist = tmp_path / "dist"
    dist.mkdir()
    deb = tmp_path / "Capsem_1.5.1_amd64.deb"

    with glowup.local_release_server(dist) as base_url:
        package_path = dist / "releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
        package_path.parent.mkdir(parents=True)
        package_path.write_bytes(b"fixture deb")
        for relative in (
            "assets/releases/2026.0709.13/x86_64-rootfs.erofs",
            "assets/releases/2026.0709.13/obom.cdx.json",
        ):
            target = dist / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(b"fixture")
        manifest_path = dist / "assets" / "nightly" / "manifest.json"
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(
            """{
  "packages": [
    {
      "name": "Capsem_1.5.1_amd64.deb",
      "url": "%s/releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
    }
  ],
  "profiles": {
    "co-work": {
      "architectures": [
        {
          "images": [
            {"url": "%s/assets/releases/2026.0709.13/x86_64-rootfs.erofs"}
          ],
          "evidence": [
            {"url": "%s/assets/releases/2026.0709.13/obom.cdx.json"}
          ]
        }
      ]
    }
  }
}
"""
            % (base_url, base_url, base_url),
            encoding="utf-8",
        )

        glowup.check_generated_release(
            base_url,
            f"{base_url}/assets/nightly/manifest.json",
            deb,
            dist,
            "nightly",
        )


def test_local_release_glowup_installed_path_asserts_channel_round_trip_and_provenance(
    monkeypatch,
) -> None:
    glowup = _load_local_release_glowup()
    calls: list[list[str]] = []

    monkeypatch.setattr(glowup, "run", lambda cmd, **_kwargs: calls.append(cmd))

    glowup.run_installed_glowup(
        install_script_url="http://127.0.0.1:1234/install.sh",
        release_base_url="http://127.0.0.1:1234",
        stable_manifest_url="http://127.0.0.1:1234/assets/stable/manifest.json",
        nightly_manifest_url="http://127.0.0.1:1234/assets/nightly/manifest.json",
        corp_manifest_url="http://127.0.0.1:1234/corp/manifest.json",
        package_version="1.5.100",
    )

    assert len(calls) == 1
    script = calls[0][-1]
    assert 'grep -F \'"package_version": "1.5.100"\'' in script
    assert 'stable_manifest_sha=$(sha256sum "$HOME/.capsem/assets/manifest.json"' in script
    assert 'test "$stable_manifest_sha" = "$stable_manifest_sha_after_switch"' in script
    assert (
        "check_update_log asset_update_complete http://127.0.0.1:1234/assets/nightly/manifest.json"
        in script
    )
    assert "CAPSEM_RELEASE_CHANNELS_URL=\"$release_channels_url\"" in script
    assert "binary_update_failed" not in script
    assert "binary_update_complete" not in script
    assert "update --yes" not in script
    assert '"package_version": "1.5.101"' not in script
    assert "check_service_installed" in script
    assert '"$HOME/.capsem/bin/capsem" status' in script
    assert 'grep -F "Installed: true"' in script
    assert 'grep -F "Running:   true"' in script
    assert 'grep -F "Service:   ok"' in script
    assert 'grep -F "Gateway:   ok"' in script
    assert "scripts/verify-installed-release.py" in script
    assert (
        "verify_installed_release http://127.0.0.1:1234/assets/stable/manifest.json stable"
        in script
    )
    assert (
        "verify_installed_release http://127.0.0.1:1234/assets/nightly/manifest.json nightly"
        in script
    )
    assert "verify_installed_release http://127.0.0.1:1234/corp/manifest.json corp" in script
    assert "service status" not in script
    assert "check_binary_versions 1.5.100" in script
    assert "CAPSEM_CHANNEL=nightly" in script
    assert "http://127.0.0.1:1234/corp/manifest.json" in script
    assert "check_update_log asset_update_complete http://127.0.0.1:1234/corp/manifest.json" in script
    assert "corporate channel is locked" in script
    assert "corp_escape_status" in script


def test_local_release_glowup_asserts_channel_isolation_and_corp_manifest() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "stable_channel_sha_before_nightly" in script
    assert "nightly channel build mutated stable manifest" in script
    assert "nightly channel build mutated stable package records" in script
    assert 'corp_manifest_url = f"{base_url}/corp/manifest.json"' in script
    assert 'corp_dir = dist / "corp"' in script
    assert "update --assets --channel nightly" in script
    assert "update --assets --channel stable" in script
    assert "check_origin_channel corp" in script


def test_local_release_glowup_forbids_metadata_only_binary_cohorts() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "rewrite_deb_version" not in script
    assert "next_patch_version" not in script
    assert "without recompiling a second binary cohort" not in script


def test_local_native_install_uses_public_manifest_contract_by_default() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    install = justfile.split(
        "install: _pnpm-install _stamp-version _check-assets _pack-initrd _materialize-config",
        maxsplit=1,
    )[1].split("\n# Run install e2e tests", maxsplit=1)[0]

    assert (
        'MANIFEST_URL="${CAPSEM_INSTALL_MANIFEST_URL:-https://release.capsem.org/assets/stable/manifest.json}"'
        in install
    )
    assert '--manifest "$MANIFEST_URL"' in install
    assert '--manifest "file://$PWD/' not in install


def test_dev_service_does_not_replace_installed_assets_with_worktree_symlink() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    ensure_body = justfile.split("_ensure-service: _sign", 1)[1].split(
        "\n# Start service daemon", 1
    )[0]

    assert "ln -sfn" not in ensure_body
    assert "assets.installed" not in ensure_body
    assert "Symlinked $ASSETS_LINK" not in ensure_body
    assert "sync-dev-assets.sh" in ensure_body
    assert "retired_config_removed" in ensure_body


def test_installers_remove_retired_user_and_service_config_rails() -> None:
    scripts = [
        PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall",
        PROJECT_ROOT / "scripts" / "deb-postinst.sh",
        PROJECT_ROOT / "scripts" / "simulate-install.sh",
    ]

    for path in scripts:
        text = path.read_text()
        assert 'retired_user_config="user"".toml"' in text
        assert '"$CAPSEM_DIR/service.toml"' in text or '"$CAPSEM_HOME_DIR/service.toml"' in text
        assert "retired_config_removed" in text


def test_installers_remove_retired_python_admin_bundle() -> None:
    scripts = [
        PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall",
        PROJECT_ROOT / "scripts" / "deb-postinst.sh",
        PROJECT_ROOT / "scripts" / "simulate-install.sh",
    ]

    for path in scripts:
        text = path.read_text()
        assert "capsem-admin-python" in text
        assert "retired_python_admin_bundle_removed" in text


def test_native_postinstall_merges_fresh_check_into_manifest_metadata() -> None:
    for relative in ("scripts/pkg-scripts/postinstall", "scripts/deb-postinst.sh"):
        script = (PROJECT_ROOT / relative).read_text()
        metadata = script.index("manifest-metadata.json")
        hydrate = script.index('update --assets --manifest \\"$MANIFEST_SOURCE\\"')
        refresh = script.index("update --check", hydrate)

        assert metadata < hydrate < refresh, relative
        assert "update-check.json" not in script, relative
        assert "update-checks" not in script, relative
        assert "update_status_refreshed" in script[refresh:], relative


def test_manifest_generation_public_path_is_capsem_admin() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    public_docs = [
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "architecture" / "asset-pipeline.md",
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "security" / "build-verification.md",
        PROJECT_ROOT / "skills" / "asset-pipeline" / "SKILL.md",
        PROJECT_ROOT / "skills" / "release-process" / "SKILL.md",
    ]

    assert "capsem-admin -- manifest generate" in justfile
    assert "scripts/gen_manifest.py" not in justfile
    assert '(cd "$ASSETS" && b3sum' not in justfile
    for path in public_docs:
        text = path.read_text()
        assert "capsem-admin manifest generate" in text
        assert "scripts/gen_manifest.py" not in text


def test_package_builders_stage_manifest_only_not_vm_asset_payload() -> None:
    build_pkg = (PROJECT_ROOT / "scripts" / "build-pkg.sh").read_text()
    repack_deb = (PROJECT_ROOT / "scripts" / "repack-deb.sh").read_text()
    deb_postinst = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()
    pkg_preinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "preinstall").read_text()
    pkg_postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()

    assert "CAPSEM_PKG_ASSET_MODE" not in build_pkg
    assert "ASSET_MODE=" not in build_pkg
    assert "export COPYFILE_DISABLE=1" in build_pkg
    assert "--manifest" in build_pkg
    assert 'MANIFEST_PATH="${2:?--manifest requires a URL}"' in build_pkg
    assert "materialize_manifest_input" not in build_pkg
    assert "materialize-package-manifest.py" not in build_pkg
    assert 'parsed.scheme not in ("http", "https", "file")' in build_pkg
    assert "urllib.request.Request(" not in build_pkg
    assert "CapsemReleaseValidator/1.0" not in build_pkg
    assert "urllib.request.urlopen" not in build_pkg
    assert "manifest must be a URL" in build_pkg
    assert "pathlib.Path(source).read_bytes()" not in build_pkg
    assert '--version "$VERSION"' in build_pkg
    assert "PKG_VERSION" not in build_pkg
    assert (
        'materialize_manifest_input "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"' not in build_pkg
    )
    assert (
        'install -m 0644 "$ASSETS_VIEW/manifest.json" "$SHARE_DIR/assets/manifest.json"'
        not in build_pkg
    )
    assert 'SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"' in build_pkg
    assert (
        'write_manifest_metadata "$SELECTED_MANIFEST_SOURCE" "$VERSION" "$SHARE_DIR/assets/manifest-metadata.json"'
        in build_pkg
    )
    assert "snapshot_sha256" not in build_pkg
    assert "materialize_manifest_assets" not in build_pkg
    assert "Added asset:" not in build_pkg
    assert "rootfs-" not in build_pkg
    assert "initrd-" not in build_pkg
    assert "vmlinuz-" not in build_pkg
    assert "obom-" not in build_pkg
    assert "sync-dev-assets.sh" not in build_pkg
    assert 'CONFIG_ROOT="${POSITIONAL[3]}"' in build_pkg
    assert 'ditto --norsrc --noextattr "$src" "$dst"' in build_pkg
    assert 'copy_tree_clean "$CONFIG_ROOT/profiles" "$SHARE_DIR/profiles"' in build_pkg
    assert 'install -m 0755 "$SCRIPT_DIR/pkg-scripts/preinstall"' in build_pkg
    assert 'xattr -rc "$WORK_DIR/payload" "$PKG_SCRIPTS"' in build_pkg
    assert 'find "$WORK_DIR/payload" "$PKG_SCRIPTS" -name' in build_pkg
    assert '--scripts "$PKG_SCRIPTS"' in build_pkg
    assert "--filter '/\\._[^/]*$'" in build_pkg
    assert "capsem-admin" in build_pkg
    assert "capsem-tui" in build_pkg
    assert "rm -rf /Applications/Capsem.app" in pkg_preinstall
    assert "event=remove_user_app_payload" in pkg_preinstall
    assert 'rm -rf "$USER_HOME/Applications/Capsem.app"' in pkg_preinstall
    assert "rm -rf /usr/local/share/capsem" in pkg_preinstall
    assert "pkill -9 -x capsem-app" in pkg_preinstall
    assert "capsem stop" not in pkg_preinstall
    assert "$CAPSEM_DIR/bin/capsem" not in pkg_preinstall
    assert "event=stop_existing_service" not in pkg_preinstall
    assert 'INSTALL_LOG="$CAPSEM_DIR/logs/install.log"' in pkg_preinstall
    assert 'INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"' in pkg_preinstall
    assert "install-current-run" in pkg_preinstall
    assert "install-latest.log" in pkg_preinstall
    assert 'exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1' in pkg_preinstall

    assert "CAPSEM_DEB_ASSET_MODE" not in repack_deb
    assert "ASSET_MODE=" not in repack_deb
    assert "export COPYFILE_DISABLE=1" in repack_deb
    assert "strip_packaged_binaries" in repack_deb
    assert "CAPSEM_REPACK_STRIP:-1" in repack_deb
    assert 'strip --strip-unneeded "$path"' in repack_deb
    assert 'CONFIG_ROOT="${POSITIONAL[2]}"' in repack_deb
    assert "--manifest" in repack_deb
    assert "materialize_manifest_input" not in repack_deb
    assert "materialize-package-manifest.py" not in repack_deb
    assert 'parsed.scheme not in ("http", "https", "file")' in repack_deb
    assert "urllib.request.Request(" not in repack_deb
    assert "CapsemReleaseValidator/1.0" not in repack_deb
    assert "urllib.request.urlopen" not in repack_deb
    assert "manifest must be a URL" in repack_deb
    assert "pathlib.Path(source).read_bytes()" not in repack_deb
    assert "BUILD_TS=" not in repack_deb
    assert (
        'materialize_manifest_input "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"' not in repack_deb
    )
    assert (
        'cp "$ASSETS_VIEW/manifest.json" "$WORK_DIR/deb/usr/share/capsem/assets/manifest.json"'
        not in repack_deb
    )
    assert 'SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"' in repack_deb
    assert 'PACKAGE_VERSION="$(dpkg-deb -f "$INPUT_DEB" Version)"' in repack_deb
    assert (
        'write_manifest_metadata "$SELECTED_MANIFEST_SOURCE" "$PACKAGE_VERSION" "$WORK_DIR/deb/usr/share/capsem/assets/manifest-metadata.json"'
        in repack_deb
    )
    assert "snapshot_sha256" not in repack_deb
    assert "materialize_manifest_assets" not in repack_deb
    assert "Added asset:" not in repack_deb
    assert "rootfs-" not in repack_deb
    assert "initrd-" not in repack_deb
    assert "vmlinuz-" not in repack_deb
    assert "obom-" not in repack_deb
    assert (
        'cp -R "$CONFIG_ROOT/profiles/." "$WORK_DIR/deb/usr/share/capsem/profiles/"' in repack_deb
    )
    assert "sync-dev-assets.sh" not in repack_deb
    assert "capsem-admin" in repack_deb
    assert "capsem-tui" in repack_deb
    assert "/usr/share/capsem/assets" in deb_postinst
    assert "/usr/share/capsem/profiles" in deb_postinst
    assert (
        'install -m 0644 /usr/share/capsem/assets/manifest.json "$CAPSEM_DIR/assets/manifest.json"'
        not in deb_postinst
    )
    assert (
        'install -m 0644 /usr/share/capsem/assets/manifest-metadata.json "$CAPSEM_DIR/assets/manifest-metadata.json"'
        in deb_postinst
    )
    assert "event=manifest_copied" not in deb_postinst
    assert "manifest check" not in deb_postinst
    assert "event=manifest_report" not in deb_postinst
    assert "MANIFEST_METADATA=$(tr" in deb_postinst
    assert "event=manifest_metadata" in deb_postinst
    assert "METADATA_MANIFEST_URL=$(sed" in deb_postinst
    assert (
        'MANIFEST_SOURCE="https://release.capsem.org/assets/stable/manifest.json"' in deb_postinst
    )
    assert "event=manifest_source" in deb_postinst
    assert (
        'CAPSEM_HOME=\\"$CAPSEM_DIR\\" CAPSEM_RUN_DIR=\\"$CAPSEM_DIR/run\\" \\"$CAPSEM_DIR/bin/capsem\\" update --assets --manifest \\"$MANIFEST_SOURCE\\"'
        in deb_postinst
    )
    assert "event=assets_hydrated" in deb_postinst
    assert "event=asset_hydration_failed" in deb_postinst
    assert "event=assets_copied" not in deb_postinst
    assert 'echo "capsem: packaged binary missing: /usr/bin/$bin" >&2' in deb_postinst
    assert "event=binary_missing bin=$bin" in deb_postinst
    assert 'INSTALL_LOG="$CAPSEM_DIR/logs/install.log"' in deb_postinst
    assert 'INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"' in deb_postinst
    assert "install-current-run" in deb_postinst
    assert "install-latest.log" in deb_postinst
    assert 'exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1' in deb_postinst
    assert 'PROFILE_COUNTS=$(echo "$STATUS_OUTPUT" | sed -n' in deb_postinst
    assert '[ "$READY_PROFILES" = "$TOTAL_PROFILES" ]' in deb_postinst
    assert '[ "$TOTAL_PROFILES" -gt 0 ]' in deb_postinst
    assert "event=profiles_not_ready" in deb_postinst
    assert "capsem-admin" in deb_postinst
    assert "capsem-tui" in deb_postinst

    assert (
        'install -m 0644 "$PKG_SHARE/assets/manifest.json" "$CAPSEM_DIR/assets/manifest.json"'
        not in pkg_postinstall
    )
    assert (
        'install -m 0644 "$PKG_SHARE/assets/manifest-metadata.json" "$CAPSEM_DIR/assets/manifest-metadata.json"'
        in pkg_postinstall
    )
    assert "event=manifest_copied" not in pkg_postinstall
    assert "manifest check" not in pkg_postinstall
    assert "event=manifest_report" not in pkg_postinstall
    assert "MANIFEST_METADATA=$(tr" in pkg_postinstall
    assert "event=manifest_metadata" in pkg_postinstall
    assert "METADATA_MANIFEST_URL=$(sed" in pkg_postinstall
    assert (
        'MANIFEST_SOURCE="https://release.capsem.org/assets/stable/manifest.json"'
        in pkg_postinstall
    )
    assert "event=manifest_source" in pkg_postinstall
    assert (
        'CAPSEM_HOME=\\"$CAPSEM_DIR\\" CAPSEM_RUN_DIR=\\"$CAPSEM_DIR/run\\" \\"$CAPSEM_DIR/bin/capsem\\" update --assets --manifest \\"$MANIFEST_SOURCE\\"'
        in pkg_postinstall
    )
    assert "event=assets_hydrated" in pkg_postinstall
    assert "event=asset_hydration_failed" in pkg_postinstall
    assert "event=assets_copied" not in pkg_postinstall
    assert 'echo "capsem: packaged binary missing: $src" >&2' in pkg_postinstall
    assert "event=binary_missing bin=$bin" in pkg_postinstall
    assert 'rm -rf "$CAPSEM_DIR"/bin.backup*' in pkg_postinstall
    assert "event=retired_binary_backups_removed" in pkg_postinstall


def test_macos_postinstall_adds_capsem_bin_to_fish_path() -> None:
    postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()

    assert ".config/fish/config.fish" in postinstall
    assert "fish_add_path" in postinstall
    assert "grep -qF 'fish_add_path --path \"$HOME/.capsem/bin\"'" in postinstall
    assert 'cp -R "$PKG_SHARE/assets/"* "$CAPSEM_DIR/assets/"' not in postinstall
    assert "pkill -x capsem-app" in postinstall
    assert 'INSTALL_LOG="$CAPSEM_DIR/logs/install.log"' in postinstall
    assert 'INSTALL_RUN_ID=$(cat "$INSTALL_RUN_FILE" 2>/dev/null || date' in postinstall
    assert 'INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"' in postinstall
    assert "install-latest.log" in postinstall
    assert 'exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1' in postinstall
    assert "event=readiness_poll" in postinstall
    assert "attempt=$attempt" in postinstall
    assert 'PROFILE_COUNTS=$(echo "$STATUS_OUTPUT" | sed -n' in postinstall
    assert '[ "$READY_PROFILES" = "$TOTAL_PROFILES" ]' in postinstall
    assert '[ "$TOTAL_PROFILES" -gt 0 ]' in postinstall
    assert "event=profiles_not_ready" in postinstall


def test_linux_postinstall_prints_service_journal_on_readiness_failure() -> None:
    postinstall = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()

    assert "event=service_diagnostics" in postinstall
    assert "systemctl --user status capsem.service --no-pager -l" in postinstall
    assert "journalctl --user-unit capsem.service --no-pager -n 100" in postinstall


def test_release_workflow_decouples_vm_assets_and_keeps_full_host_binary_set() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    qualification = (
        PROJECT_ROOT / ".github" / "workflows" / "release-qualification.yaml"
    ).read_text()

    assert "  build-assets:" not in workflow
    assert "vm-assets-" not in workflow
    assert "assets/current" not in workflow
    assert """echo '{"releases":{}}'""" not in workflow
    assert "Complete canonical release gate (just test)" in qualification
    assert "run: just test" not in workflow
    assert "scripts/check-release-qualification.py" in workflow
    assert "just build-kernel" not in workflow
    assert "just build-rootfs" not in workflow
    assert "RELEASE_CHANNEL: ${{ inputs.channel }}" in workflow
    assert (
        "ASSET_MANIFEST_URL: https://release.capsem.org/assets/${{ inputs.channel }}/manifest.json"
        in workflow
    )
    assert '--manifest "$ASSET_MANIFEST_URL"' in workflow
    assert "-p capsem-admin" in workflow


def test_release_workflow_retries_app_cargo_tool_installs() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    build_app_macos = workflow.split("  build-app-macos:", 1)[1].split("\n  build-app-linux:", 1)[0]
    build_app_linux = workflow.split("  build-app-linux:", 1)[1].split("\n  create-release:", 1)[0]

    assert "cargo install tauri-cli cargo-auditable cargo-sbom --locked" not in workflow
    assert "cargo install tauri-cli cargo-auditable --locked" not in workflow

    for block, required_tools in (
        (build_app_macos, ("tauri-cli", "cargo-auditable")),
        (build_app_linux, ("tauri-cli", "cargo-auditable")),
    ):
        assert "CARGO_NET_RETRY: 10" in block
        assert "install_cargo_tool() {" in block
        assert "for attempt in 1 2 3; do" in block
        assert 'cargo install "$tool" --locked' in block
        assert 'echo "cargo install $tool failed on attempt $attempt/3"' in block
        for tool in required_tools:
            assert f"install_cargo_tool {tool}" in block
    assert "cargo install cargo-sbom --locked" in build_app_macos
    assert "cargo install cargo-sbom --locked" not in build_app_linux
    assert "install_cargo_tool cargo-sbom" not in workflow
    assert "-p capsem-tui" in workflow
    assert "-p capsem-mcp-aggregator" in workflow
    assert "-p capsem-mcp-builtin" in workflow
    assert "capsem-admin" in workflow
    assert "capsem-tui" in workflow
    assert "capsem-mcp-aggregator" in workflow
    assert "capsem-mcp-builtin" in workflow


def test_release_workflow_sets_up_uv_before_uv_run_steps() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    jobs_with_uv = {
        name: block for name, block in _workflow_job_blocks(workflow).items() if "uv run" in block
    }

    assert jobs_with_uv
    for name, block in jobs_with_uv.items():
        setup_pos = block.find("astral-sh/setup-uv@v5")
        uv_run_pos = block.find("uv run")
        assert setup_pos != -1, f"{name} uses uv run without setup-uv"
        assert setup_pos < uv_run_pos, f"{name} sets up uv after first uv run"


def test_asset_build_recipes_skip_kvm_only_for_build_prereq_doctor() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    doctor_linux = (PROJECT_ROOT / "scripts" / "doctor-linux.sh").read_text()

    assert "CAPSEM_SKIP_KVM_CHECK" in doctor_linux
    assert 'skip "/dev/kvm (CAPSEM_SKIP_KVM_CHECK set)"' in doctor_linux

    for recipe in ("build-kernel", "build-rootfs", "build-assets"):
        block = justfile.split(f"\n{recipe} ", 1)[1].split("\n# ", 1)[0]
        assert "CAPSEM_SKIP_ASSET_CHECK=1 CAPSEM_SKIP_KVM_CHECK=1 just doctor" in block

    smoke_block = justfile.split("\nsmoke", 1)[1].split("\n# ", 1)[0]
    assert "CAPSEM_SKIP_KVM_CHECK" not in smoke_block


def test_only_systemd_package_proof_receives_kvm_devices() -> None:
    cross_compile = _just_recipe_block("cross-compile")
    proof = _just_recipe_block("_prove-linux-deb")

    assert "DOCKER_KVM_ARGS" not in cross_compile
    assert "--device /dev/kvm" not in cross_compile
    assert "--device /dev/vhost-vsock" not in cross_compile
    assert "DEVICE_ARGS=(" in proof
    assert "--device /dev/kvm" in proof
    assert "--device /dev/vhost-vsock" in proof
    assert '"${DEVICE_ARGS[@]}"' in proof


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
