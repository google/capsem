"""Profile-owned asset build rail tests."""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent

#: `resources()` takes the runner it should build with; these tests ask
#: *what* is held, so any runner will do.
def _resource_runner():
    from helpers.gate import RecordingRunner

    return RecordingRunner(PROJECT_ROOT)


RUNNER_FOR_RESOURCES = _resource_runner()


def _source_text(relative: str) -> str:
    return (PROJECT_ROOT / relative).read_text(encoding="utf-8")


def _planned(command: str, **args) -> str:
    """What a command's plan would run, rendered.

    These contracts were written against recipe bodies. The recipes are
    dispatches now, so the same claims are read from the plan -- which is the
    stronger question: a text search notices a line that stopped being written,
    while this notices a step that stopped running.
    """
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand

    return (
        GateCommand.registry[command](
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
        )
        ._describe()
        .describe()
    )


def _recipe_block(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line == name or line.startswith(f"{name} "))
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    block = "\n".join(lines[start:end])
    if name == "test:":
        block = f"{block}\n{_recipe_block('_test-candidate:')}"
    return block


def test_build_assets_requires_profile_and_uses_capsem_admin() -> None:
    """Every image build names the profile whose manifest it is building from.

    The shell guarded this with `if [[ -z "$PROFILE_ARG" ]]`, which could only
    catch an empty string. Building the argv from the profile means an image
    build without one cannot be expressed.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.imagebuild import build_argv

    config = gate_config.load(PROJECT_ROOT)
    argv = build_argv(config, profile="code", arch="arm64", template="all")

    assert argv[: len(config.imagebuild.admin)] == list(config.imagebuild.admin)
    assert "cargo" in argv[0] or "capsem-admin" in " ".join(argv)
    assert "--profile" in argv
    assert argv[argv.index("--profile") + 1] == "config/profiles/code/profile.toml"
    assert "uv run capsem-builder build guest/" not in " ".join(argv)


def test_asset_build_primitives_accept_an_isolated_output_root() -> None:
    """And the value reaches the builder, which is where it used to be lost.

    `_build-image-template` declared an `output` parameter and never forwarded
    it, so the builder wrote to the one configured tree while each concurrent
    lane verified a private directory nothing had written. This contract was
    right and the recipe was wrong; asserting on the argv the builder receives
    is what makes the difference visible.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.imagebuild import build_argv

    config = gate_config.load(PROJECT_ROOT)

    default = build_argv(config, profile="code", arch="arm64", template="all")
    assert default[default.index("--output") + 1] == config.imagebuild.output

    isolated = build_argv(
        config, profile="code", arch="arm64", template="all", output="/tmp/lane-a"
    )
    assert isolated[isolated.index("--output") + 1] == "/tmp/lane-a"


def test_just_test_owns_the_complete_asset_build_and_boot_gate() -> None:
    """Every profile, both architectures, built and then booted.

    Read out of the recipe text when this was shell. The steps are now asserted
    against the commands the gate issues, in tests/test_gate_assets.py; what
    stays here is that `just test` still owns the gate and that the gate still
    does each of these things at all.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    assets = _source_text("src/capsem/gate/assets.py")
    lanes = _source_text("src/capsem/gate/assetlanes.py")

    # `just test` still owns the gate -- as a composed phase now rather than a
    # recipe that dispatched to another recipe, so it is read from the plan.
    assert "artifacts.assets" in _planned("candidate")

    # Profiles are discovered, not listed.
    assert config.assets.profiles_glob == "config/profiles/*/profile.toml"
    assert "profiles_glob" in lanes

    # Both image stages, per architecture. The stage list is config now, so
    # this reads it rather than repeating it.
    assert config.imagebuild.lane_templates == ("kernel", "rootfs")
    assert "for stage in self._config.imagebuild.lane_templates" in lanes

    # `current` is repointed by whichever lane finished last, and the
    # host-architecture VM proof that follows needs it aimed at this machine.
    assert "current.symlink_to(self.host_arch.name)" in assets
    assert "current.readlink()" in assets

    # Hash aliases before the manifest check, or startup falls through to a
    # remote fetch for a local-only asset version and the gate stops being
    # hermetic.
    assert assets.index("hash_assets_script") < assets.index('"manifest", "check"')

    # AF_UNIX paths must stay under macOS SUN_LEN once the gateway appends its
    # session path, so the run dir lives outside the descriptive scratch root.
    assert config.assets.run_dir_template.startswith("/tmp/")
    assert "tempfile.mkdtemp" in assets

    assert "shell_proof_script" in assets
    assert config.assets.shell_proof_script.endswith("prove-installed-shell.py")


def test_a_failed_boot_preserves_only_host_side_evidence() -> None:
    """A blanket copy also takes the guest's workspace into target/.

    The snapshots duplicate that workspace once per generation, and the same
    name filter is what keeps the VM disk image and session.db out.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)

    assert set(config.assets.evidence_prune_dirs) == {"guest", "auto_snapshots"}
    assert ".log" in config.assets.evidence_suffixes
    assert ".toml" in config.assets.evidence_suffixes, (
        "vm/active_profile.toml records the asset pins a hash mismatch is "
        "argued from"
    )


def test_asset_gate_runs_architecture_lanes_in_parallel_before_boot_proofs() -> None:
    """Both lanes complete before anything merges or boots.

    A hosted release runner has an observed hard lifetime below the workflow's
    nominal timeout, so the four-cell matrix only fits if the architectures
    build concurrently -- and merging before both finish would publish a
    manifest for assets that do not exist yet.
    """
    assets = _source_text("src/capsem/gate/assets.py")
    lanes = _source_text("src/capsem/gate/assetlanes.py")

    assert "ThreadPoolExecutor" in lanes
    assert "lanes.run(" in assets
    assert assets.index("lanes.run(") < assets.index("self._merge_lanes(")
    assert assets.index("self._merge_lanes(") < assets.index("self._prove(")


def test_asset_gate_reaps_gateway_and_service_between_profile_proofs() -> None:
    """Each profile's daemons stop before the next profile starts.

    The gateway goes first: it owns the fixed localhost port, and one that
    outlives its service attaches the next profile to a UDS pointing at a run
    directory that has already been deleted.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    assets = _source_text("src/capsem/gate/assets.py")

    assert config.pidfiles.names == ("gateway.pid", "service.pid")
    assert "pidfiles.stop_gate_service" in assets
    # In the `finally`, so an aborted proof still reaps.
    assert assets.index("finally:", assets.index("def _prove(")) < assets.index(
        "shutil.rmtree(run_dir"
    )


def test_asset_ci_uses_primitives_owned_by_just_test() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    lanes = _source_text("src/capsem/gate/assetlanes.py")

    assert 'just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"' in workflow
    assert 'just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"' in workflow
    assert "_build-image-template" in lanes


def test_asset_ci_installs_pinned_pnpm_before_running_build_primitives() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    build_assets = workflow.split("\n  build-assets:\n", maxsplit=1)[1].split(
        "\n  reuse-assets:", maxsplit=1
    )[0]
    pnpm_setup = (
        "- uses: pnpm/action-setup@fc06bc1257f339d1d5d8b3a19a8cae5388b55320\n"
        "        with:\n"
        "          version: 10"
    )

    assert pnpm_setup in build_assets
    assert build_assets.index(pnpm_setup) < build_assets.index(
        "actions/setup-node@a0853c24544627f65ddf259abe73b1d18a591444"
    )
    assert build_assets.index(pnpm_setup) < build_assets.index("just _build-kernel")


def test_asset_matrix_preflights_once_and_reuses_the_public_build_primitive() -> None:
    """One builder invocation, reused per stage, preflighted once.

    The lanes used to reach it through `just _build-image-template`, so this
    asserted the dispatch text. They call `build_argv` directly now -- the same
    single spelling, without a second gate process between the lane and the
    builder.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.imagebuild import build_argv

    config = gate_config.load(PROJECT_ROOT)
    lanes = (PROJECT_ROOT / "src/capsem/gate/assetlanes.py").read_text(encoding="utf-8")

    # Every stage the lanes build goes through the one primitive.
    assert "imagebuild.build_argv(" in lanes
    for stage in config.imagebuild.lane_templates:
        argv = build_argv(config, profile="code", arch="arm64", template=stage)
        assert argv[argv.index("--template") + 1] == stage

    # The primitive builds; it does not preflight. Preflighting per stage is
    # what made a four-cell matrix run doctor four times.
    assert "install-tools" not in " ".join(build_argv(
        config, profile="code", arch="arm64", template="all"
    ))
    assert "doctor" not in lanes


def test_check_assets_recovers_by_iterating_checked_in_profiles() -> None:
    """Every checked-in profile, discovered rather than named.

    The shell globbed `config/profiles/*/profile.toml`; `imagebuild.profiles`
    does the same glob from config and raises when it matches nothing, so a
    checkout with no profiles fails loudly instead of building none of them.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.imagebuild import profiles

    config = gate_config.load(PROJECT_ROOT)
    found = profiles(config)

    assert len(found) > 1, "the recovery path must cover every profile, not one"
    assert "code" in found
    lanes = (PROJECT_ROOT / "src/capsem/gate/imagebuild.py").read_text(encoding="utf-8")
    assert 'profile="code"' not in lanes, "a profile is named rather than discovered"


def test_in_container_commands_write_only_where_the_container_user_owns() -> None:
    """/src is a bind mount of the host checkout. On Linux the host UID does not
    own it, so anything `docker exec -u capsem` writes outside an explicitly
    chowned path fails with EACCES -- and macOS maps the mount cleanly, so only
    CI ever sees it. Four separate release-gate failures came from this one
    shape: the builder's git, the staging rm, pytest's cache, and the
    unmaterialized profile catalog."""
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    guest = config.install.guest_user
    container = (
        PROJECT_ROOT / "src" / "capsem" / "gate" / "installcontainer.py"
    ).read_text()
    proof = (PROJECT_ROOT / "src" / "capsem" / "gate" / "installproof.py").read_text()

    # Removing target/install-test-* needs write permission on their parent.
    # Granted as the one directory entry: recursive here would walk every
    # cargo artifact in the checkout.
    assert 'f"{owner}:{owner}", f"{self._settings.mount}/{target}"' in container
    assert '"-R", f"{owner}:{owner}", f"{self._settings.mount}/{target}"' not in container
    # The directory entry, taken from the layout rather than spelled again.
    assert "Path(self._settings.layout.assets).parts[0]" in container

    # Every path this user writes has to live off the bind mount.
    for path in (guest.tmp, guest.pytest_cache, guest.asset_manifest, config.install.venv):
        assert path.startswith(guest.home), (
            f"{path} is not under the container user's home, so it may land on "
            "the bind mount and fail with EACCES on Linux"
        )
        assert not path.startswith(config.install.mount)

    assert "TMPDIR" in proof and "guest.tmp" in proof
    assert "cache_dir=" in proof and "pytest_cache" in proof


def test_runtime_recipes_materialize_generated_config_before_service() -> None:
    # `_prepared-runtime` owns this sequence for every runtime entry point, so
    # the ordering is asserted once where it lives rather than re-checked in
    # each caller. Four copies of it is how a fifth caller drops a step.
    prepared = _recipe_block("_prepared-runtime:")
    assert "_pack-initrd" in prepared
    assert "_materialize-config" in prepared
    assert prepared.index("_pack-initrd") < prepared.index("_materialize-config")

    for recipe in ["shell:", "run-service:", "smoke:"]:
        assert "_prepared-runtime" in _recipe_block(recipe)


def test_materialize_config_uses_admin_profile_command() -> None:
    block = _recipe_block("_materialize-config:")

    assert "scripts/materialize-config.sh" in block

    script = (PROJECT_ROOT / "scripts" / "materialize-config.sh").read_text()
    assert "cargo run -p capsem-admin -- profile materialize" in script
    assert "normalize_arch()" in script
    assert 'case "$arch" in' in script
    assert "arm64|aarch64)" in script
    assert "--config-root" in script
    assert "--manifest" in script
    assert "--output-root" in script
    assert "target/config" in script


def test_materialize_config_falls_back_to_sole_manifest_arch_for_ci_runner() -> None:
    script = (PROJECT_ROOT / "scripts" / "materialize-config.sh").read_text()

    assert 'manifest["assets"]["current"]' in script
    assert 'manifest["assets"]["releases"][current]["arches"]' in script
    assert 'if [ "$arch_source" = "host" ] && [ "$manifest_arch_count" = "1" ]; then' in script
    assert "using sole manifest arch" in script
    assert 'arch_source="CAPSEM_ARCH"' in script
    assert "materialize arch $arch from $arch_source is not present" in script


def test_materialize_config_uses_release_manifest_profile_membership() -> None:
    block = _recipe_block("_materialize-config:")
    script = (PROJECT_ROOT / "scripts" / "materialize-config.sh").read_text()

    assert 'rm -rf "$OUTPUT_ROOT"' in script
    assert 'rm -rf "$ROOT/target/config"' not in script
    assert 'manifest_schema="release"' in script
    assert 'profile_ids="$(' in script
    assert 'profile_path="$CONFIG_ROOT/profiles/$profile_id/profile.toml"' in script
    assert "selected release profile source is missing" in script
    assert 'profile_paths=("$CONFIG_ROOT"/profiles/*/profile.toml)' in script
    assert 'for profile_path in "${profile_paths[@]}"; do' in script
    assert '--profile "$profile_path"' in script
    assert '--profile "$ROOT/config/profiles/code/profile.toml"' not in script
    assert "scripts/materialize-config.sh" in block


def test_ensure_service_uses_generated_profiles() -> None:
    """The daemon reads materialized profiles, and says so when they are absent.

    A service started against the checked-in sources would boot profiles that
    had never been through `capsem-admin profile materialize`, which is a
    different product from the one being tested.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    service = (PROJECT_ROOT / "src/capsem/gate/service.py").read_text(encoding="utf-8")

    assert config.service.generated_profiles == "target/config/profiles"
    assert "CAPSEM_PROFILES_DIR" in service
    assert "generated profiles are missing" in service


def test_isolated_test_recipes_trap_test_home_service_cleanup() -> None:
    """Every isolated run stops the service in its own home, by pidfile.

    Two recipes each carried an EXIT trap and a hand-written pidfile read. A
    trap is correct only for the commands inside it; `Workspace` is a
    `Resource`, so the stop happens on every path including the aborted one,
    and for every command that holds one rather than the two that remembered.

    Never by pattern: `pkill -f` takes down a developer's installed capsem, or
    a parallel run with a different `CAPSEM_HOME`.
    """
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate import config as gate_config
    from capsem.gate.command import GateCommand

    workspace_source = (
        PROJECT_ROOT / "src/capsem/gate/workspace.py"
    ).read_text(encoding="utf-8")
    pidfile_source = (
        PROJECT_ROOT / "src/capsem/gate/pidfiles.py"
    ).read_text(encoding="utf-8")

    assert "stop_gate_service" in workspace_source
    assert gate_config.load(PROJECT_ROOT).pidfiles.names == ("gateway.pid", "service.pid")
    assert "pkill" not in pidfile_source and "killall" not in pidfile_source

    for name in ("candidate", "smoke"):
        command = GateCommand.registry[name](
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False),
        )
        held = {resource.name for resource in command.resources(RUNNER_FOR_RESOURCES)}
        assert "workspace" in held, f"{name} runs outside an isolated home"


def test_release_workflow_uses_same_config_materializer() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()

    assert workflow.count("bash scripts/materialize-config.sh") >= 2
    assert workflow.count('CAPSEM_ASSET_MANIFEST="$PREACTIVATION_MANIFEST"') >= 2
    assert 'CAPSEM_ARCH="${{ matrix.arch }}"' in workflow


def test_asset_workflow_publishes_obom_not_debug_build_ledger() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    release = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    stager = (PROJECT_ROOT / "scripts/stage-profile-publication.py").read_text()

    assert "npm install -g @cyclonedx/cdxgen@12.7.0" in workflow
    assert "@cyclonedx/cdxgen@latest" not in workflow
    assert "CAPSEM_CDXGEN_CMD: cdxgen" in workflow
    stage_step = workflow.split(
        "- name: Stage and verify immutable profile publication once", maxsplit=1
    )[1].split(
        "\n      - uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
        maxsplit=1,
    )[0]
    upload_step = workflow.split("- name: Publish immutable GitHub profile release", maxsplit=1)[
        1
    ].split("\n      - name: Attest VM asset provenance", maxsplit=1)[0]
    attest_step = workflow.split("- name: Attest VM asset provenance", maxsplit=1)[1].split(
        "\n      - uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
        maxsplit=1,
    )[0]
    assert "scripts/stage-profile-publication.py" in stage_step
    assert "scripts/verify-profile-publication.py" in stage_step
    assert "scripts/stage-profile-publication.py" not in upload_step
    assert "scripts/verify-profile-publication.py" in upload_step
    assert "scripts/publish-immutable-release-assets.sh" in upload_step
    assert "gh release create" not in upload_step
    assert "gh release upload" not in upload_step
    assert 'files=("$RELEASE_DIR"/*)' in upload_step
    assert 'for section in ("config", "images", "evidence")' in stager
    assert "subject-path: target/asset-release/profile-*/*" in attest_step
    assert "vm-build-ledger-" not in workflow
    assert "build-ledger.log" not in upload_step
    assert "build-ledger.log" not in attest_step
    assert "B3SUMS" not in upload_step
    assert "B3SUMS" not in attest_step
    assert "obom.cdx.json" not in release
    assert "Skipping debug-only $arch/$base from release upload" not in release
