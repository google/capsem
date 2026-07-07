"""Install package asset-payload contract tests."""

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


def test_just_install_does_not_sync_assets_after_installer() -> None:
    install_body = _just_recipe_block("install:")

    assert "Syncing local dev assets" not in install_body
    assert "scripts/sync-dev-assets.sh" not in install_body
    assert "CAPSEM_PKG_ASSET_MODE=current-arch bash scripts/build-pkg.sh" not in install_body
    assert "CAPSEM_DEB_ASSET_MODE=current-arch bash scripts/repack-deb.sh" not in install_body
    assert "bash scripts/build-pkg.sh" in install_body
    assert "bash scripts/repack-deb.sh --manifest" in install_body
    assert '--manifest "file://$PWD/{{assets_dir}}/manifest.json"' in install_body
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


def test_cross_compile_repacks_deb_before_boot_test() -> None:
    block = _just_recipe_block("cross-compile")

    companion_pos = block.find("--- Build companion host binaries ---")
    tauri_pos = block.find("cargo tauri build --target")
    repack_pos = block.find("scripts/repack-deb.sh")
    validate_pos = block.find("dpkg-deb --contents")
    install_pos = block.find('dpkg -i \\"\\$DEB\\"')
    capsem_home_pos = block.find('CAPSEM_HOME=\\"\\$BOOT_CAPSEM_HOME\\"')
    doctor_pos = block.find("--binary /usr/bin/capsem")

    assert companion_pos != -1
    assert tauri_pos != -1
    assert repack_pos != -1
    assert validate_pos != -1
    assert install_pos != -1
    assert capsem_home_pos != -1
    assert doctor_pos != -1
    assert companion_pos < tauri_pos < repack_pos < validate_pos < install_pos < capsem_home_pos < doctor_pos
    assert "--security-opt seccomp=unconfined --device /dev/kvm" in block
    assert "--device /dev/vhost-vsock" in block
    assert "--device /dev/vsock" in block
    assert "capsem-admin)\\$'" in block
    assert "-e \"HOST_UID=$HOST_UID\"" in block
    assert "-e \"HOST_GID=$HOST_GID\"" in block
    assert "trap 'chown -R \\\"\\$HOST_UID:\\$HOST_GID\\\"" in block
    assert "/src/frontend/node_modules /src/frontend/dist" in block
    assert 'dpkg -i /cargo-target/$RUST_TARGET/release/bundle/deb/*.deb' not in block


def test_install_test_restores_host_workspace_ownership() -> None:
    block = _just_recipe_block("test-install")

    assert "HOST_UID=$(id -u)" in block
    assert "HOST_GID=$(id -g)" in block
    assert 'chown -R $HOST_UID:$HOST_GID /src' in block
    assert "trap cleanup EXIT" in block
    assert "docker rm -f \"$CONTAINER\"" in block


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
    assert "materialize_manifest_input" in build_pkg
    assert 'parsed.scheme in ("http", "https")' in build_pkg
    assert "urllib.request.Request(" in build_pkg
    assert 'headers={"User-Agent": "CapsemReleaseValidator/1.0"}' in build_pkg
    assert "urllib.request.urlopen(request, timeout=60)" in build_pkg
    assert "unsupported manifest URL scheme" in build_pkg
    assert "manifest must be a URL" in build_pkg
    assert "pathlib.Path(source).read_bytes()" not in build_pkg
    assert '--version "$VERSION"' in build_pkg
    assert "PKG_VERSION" not in build_pkg
    assert 'materialize_manifest_input "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"' in build_pkg
    assert (
        'install -m 0644 "$ASSETS_VIEW/manifest.json" "$SHARE_DIR/assets/manifest.json"'
        in build_pkg
    )
    assert 'SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"' in build_pkg
    assert (
        'write_manifest_origin "$SELECTED_MANIFEST_SOURCE" "$SHARE_DIR/assets/manifest.json" "$VERSION" "$SHARE_DIR/assets/manifest-origin.json"'
        in build_pkg
    )
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
    assert 'CAPSEM_REPACK_STRIP:-1' in repack_deb
    assert 'strip --strip-unneeded "$path"' in repack_deb
    assert 'CONFIG_ROOT="${POSITIONAL[2]}"' in repack_deb
    assert "--manifest" in repack_deb
    assert "materialize_manifest_input" in repack_deb
    assert 'parsed.scheme in ("http", "https")' in repack_deb
    assert "urllib.request.Request(" in repack_deb
    assert 'headers={"User-Agent": "CapsemReleaseValidator/1.0"}' in repack_deb
    assert "urllib.request.urlopen(request, timeout=60)" in repack_deb
    assert "unsupported manifest URL scheme" in repack_deb
    assert "manifest must be a URL" in repack_deb
    assert "pathlib.Path(source).read_bytes()" not in repack_deb
    assert "BUILD_TS=" not in repack_deb
    assert 'materialize_manifest_input "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"' in repack_deb
    assert (
        'cp "$ASSETS_VIEW/manifest.json" "$WORK_DIR/deb/usr/share/capsem/assets/manifest.json"'
        in repack_deb
    )
    assert 'SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"' in repack_deb
    assert 'PACKAGE_VERSION="$(dpkg-deb -f "$INPUT_DEB" Version)"' in repack_deb
    assert (
        'write_manifest_origin "$SELECTED_MANIFEST_SOURCE" "$WORK_DIR/deb/usr/share/capsem/assets/manifest.json" "$PACKAGE_VERSION" "$WORK_DIR/deb/usr/share/capsem/assets/manifest-origin.json"'
        in repack_deb
    )
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
        in deb_postinst
    )
    assert (
        'install -m 0644 /usr/share/capsem/assets/manifest-origin.json "$CAPSEM_DIR/assets/manifest-origin.json"'
        in deb_postinst
    )
    assert "event=manifest_copied" in deb_postinst
    assert (
        'MANIFEST_REPORT=$(/usr/bin/capsem-admin manifest check --json "$CAPSEM_DIR/assets/manifest.json" | tr'
        in deb_postinst
    )
    assert "event=manifest_report" in deb_postinst
    assert "MANIFEST_ORIGIN=$(tr" in deb_postinst
    assert "event=manifest_origin" in deb_postinst
    assert (
        'CAPSEM_HOME=\\"$CAPSEM_DIR\\" CAPSEM_RUN_DIR=\\"$CAPSEM_DIR/run\\" \\"$CAPSEM_DIR/bin/capsem\\" update --assets'
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
    assert "capsem-admin" in deb_postinst
    assert "capsem-tui" in deb_postinst

    assert (
        'install -m 0644 "$PKG_SHARE/assets/manifest.json" "$CAPSEM_DIR/assets/manifest.json"'
        in pkg_postinstall
    )
    assert (
        'install -m 0644 "$PKG_SHARE/assets/manifest-origin.json" "$CAPSEM_DIR/assets/manifest-origin.json"'
        in pkg_postinstall
    )
    assert "event=manifest_copied" in pkg_postinstall
    assert (
        'MANIFEST_REPORT=$("$CAPSEM_DIR/bin/capsem-admin" manifest check --json "$CAPSEM_DIR/assets/manifest.json" | tr'
        in pkg_postinstall
    )
    assert "event=manifest_report" in pkg_postinstall
    assert "MANIFEST_ORIGIN=$(tr" in pkg_postinstall
    assert "event=manifest_origin" in pkg_postinstall
    assert (
        'CAPSEM_HOME=\\"$CAPSEM_DIR\\" CAPSEM_RUN_DIR=\\"$CAPSEM_DIR/run\\" \\"$CAPSEM_DIR/bin/capsem\\" update --assets'
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


def test_release_workflow_decouples_vm_assets_and_keeps_full_host_binary_set() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()

    assert "  build-assets:" not in workflow
    assert "vm-assets-" not in workflow
    assert "assets/current" not in workflow
    assert """echo '{"releases":{}}'""" not in workflow
    assert "Create stub v2 asset manifest for unit tests" in workflow
    assert "just build-kernel" not in workflow
    assert "just build-rootfs" not in workflow
    assert "ASSET_MANIFEST_URL: https://release.capsem.org/assets/stable/manifest.json" in workflow
    assert '--manifest "$ASSET_MANIFEST_URL"' in workflow
    assert "-p capsem-admin" in workflow


def test_release_workflow_retries_app_cargo_tool_installs() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    build_app_macos = workflow.split("  build-app-macos:", 1)[1].split("\n  build-app-linux:", 1)[0]
    build_app_linux = workflow.split("  build-app-linux:", 1)[1].split("\n  create-release:", 1)[0]

    assert "cargo install tauri-cli cargo-auditable cargo-sbom --locked" not in workflow
    assert "cargo install tauri-cli cargo-auditable --locked" not in workflow

    for block, required_tools in (
        (build_app_macos, ("tauri-cli", "cargo-auditable", "cargo-sbom")),
        (build_app_linux, ("tauri-cli", "cargo-auditable")),
    ):
        assert "CARGO_NET_RETRY: 10" in block
        assert "install_cargo_tool() {" in block
        assert "for attempt in 1 2 3; do" in block
        assert 'cargo install "$tool" --locked' in block
        assert 'echo "cargo install $tool failed on attempt $attempt/3"' in block
        for tool in required_tools:
            assert f"install_cargo_tool {tool}" in block
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
