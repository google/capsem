"""The runtime asset build rail: one runtime, both architectures."""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[3]


#: `resources()` takes the runner it should build with; these tests ask
#: *what* is held, so any runner will do.
def _resource_runner():
    from helpers.gate import RecordingRunner

    return RecordingRunner(PROJECT_ROOT)


RUNNER_FOR_RESOURCES = _resource_runner()


def _source_text(relative: str) -> str:
    return (PROJECT_ROOT / relative).read_text(encoding="utf-8")


def _command(command: str, **args):
    import argparse

    from capsem_builder.gate import cli  # noqa: F401 - registers every command
    from capsem_builder.gate.command import GateCommand
    from helpers.gate import RecordingRunner

    return GateCommand.registry[command](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
    )


def _planned(command: str, **args) -> str:
    """What a command's plan would run, rendered.

    These contracts were written against recipe bodies. The recipes are
    dispatches now, so the same claims are read from the plan -- which is the
    stronger question: a text search notices a line that stopped being written,
    while this notices a step that stopped running.
    """
    return _command(command, **args)._describe().describe()


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


def test_build_assets_uses_capsem_admin() -> None:
    """An image build is a function of the architecture and the template alone."""
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.imagebuild import build_argv

    config = gate_config.load(PROJECT_ROOT)
    argv = build_argv(config, arch="arm64", template="all")

    assert argv[: len(config.imagebuild.admin)] == list(config.imagebuild.admin)
    assert argv[argv.index("--config-root") + 1] == config.imagebuild.config_root
    assert argv[argv.index("--arch") + 1] == "arm64"
    assert "uv run --project build_system --frozen capsem-builder build guest/" not in " ".join(
        argv
    )


def test_every_asset_build_rail_materializes_exact_bases_before_building() -> None:
    """Standalone asset CI and the complete gate share the cold-host edge."""
    for command, args in (
        ("build-assets", {"arch": "arm64", "template": "all"}),
        ("assets", {}),
    ):
        rendered = _planned(command, **args)
        assert "materialize exact guest base images" in rendered
        assert "materialize locked guest Rust builders" in rendered
        following = "image build" if command == "build-assets" else "run container preflight"
        assert rendered.index("materialize exact guest base images") < rendered.index(following)
        if command == "build-assets":
            assert "prove Docker can execute arm64 containers" in rendered
            assert rendered.index("prove Docker can execute arm64 containers") < rendered.index(
                "materialize locked guest Rust builders"
            )
            assert rendered.index("materialize locked guest Rust builders") < rendered.index(
                following
            )
        else:
            assert rendered.index(following) < rendered.index(
                "materialize locked guest Rust builders"
            )


def test_kernel_only_asset_build_does_not_materialize_a_rust_helper() -> None:
    rendered = _planned("build-assets", arch="arm64", template="kernel")

    assert "prove Docker can execute arm64 containers" in rendered
    assert "Rust builder bases (none)" in rendered
    assert "materialize locked guest Rust builders" not in rendered
    assert "repack" not in rendered


def test_rootfs_asset_build_repacks_then_regenerates_its_manifest() -> None:
    command = _command("build-assets", arch="arm64", template="rootfs")
    plan = command.plan()

    assert plan.after_of("pack-initrds") == {"image.rootfs.arm64"}
    assert plan.after_of("manifest") == {"pack-initrds"}
    assert plan.after_of("hash-aliases") == {"manifest"}
    packed = plan.step_named("pack-initrds")
    assert packed.produces == (
        PROJECT_ROOT / "cache" / "target" / "assets" / "arm64" / "initrd.img",
    )
    rendering = "\n".join(packed.render())
    assert "--arch arm64" in rendering
    assert str(packed.produces[0]) in rendering


def test_unscoped_rootfs_asset_build_repacks_every_architecture() -> None:
    from capsem_builder.gate import config as gate_config

    plan = _command("build-assets", arch=None, template="all").plan()
    packed = plan.step_named("pack-initrds")

    assert {path.parent.name for path in packed.produces} == set(
        gate_config.load(PROJECT_ROOT).architectures
    )


def test_standalone_asset_build_proves_execution_before_helper_materialization() -> None:
    command = _command(
        "build-assets",
        arch="arm64",
        template="rootfs",
    )
    plan = command.plan()

    assert plan.after_of("doctor") == {"base-images"}
    assert plan.after_of("guest-execution") == {"doctor"}
    assert plan.after_of("guest-builders") == {"guest-execution"}
    assert plan.after_of("asset-tools") == {"guest-builders"}
    assert plan.after_of("asset-dependencies") == {"asset-tools"}
    assert plan.after_of("image.rootfs.arm64") == {"asset-dependencies"}


def test_rootfs_materializes_asset_tools_before_repack() -> None:
    plan = _command("build-assets", arch="x86_64", template="rootfs").plan()

    assert plan.after_of("guest-builders") == {"guest-execution"}
    assert plan.after_of("asset-tools") == {"guest-builders"}
    assert plan.after_of("asset-dependencies") == {"asset-tools"}
    assert plan.after_of("image.rootfs.x86_64") == {"asset-dependencies"}
    assert plan.after_of("pack-initrds") == {"image.rootfs.x86_64"}


def test_asset_build_primitives_accept_an_isolated_output_root() -> None:
    """And the value reaches the builder, which is where it used to be lost.

    `_build-image-template` declared an `output` parameter and never forwarded
    it, so the builder wrote to the one configured tree while each concurrent
    lane verified a private directory nothing had written. This contract was
    right and the recipe was wrong; asserting on the argv the builder receives
    is what makes the difference visible.
    """
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.imagebuild import build_argv

    config = gate_config.load(PROJECT_ROOT)

    default = build_argv(config, arch="arm64", template="all")
    assert default[default.index("--output") + 1] == config.imagebuild.output

    isolated = build_argv(config, arch="arm64", template="all", output="/tmp/lane-a")
    assert isolated[isolated.index("--output") + 1] == "/tmp/lane-a"


def test_just_test_owns_the_complete_asset_build_and_boot_gate() -> None:
    """The one runtime, both architectures, built and then booted.

    Read out of the recipe text when this was shell. The steps are now asserted
    against the commands the gate issues, in build_system/tests/gate/test_gate_assets.py; what
    stays here is that `just test` still owns the gate and that the gate still
    does each of these things at all.
    """
    from capsem_builder.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    assets = _source_text("build_system/builder/gate/assets.py")
    lanes = _source_text("build_system/builder/gate/assetlanes.py")

    # `just test` still owns the gate -- as a composed phase now rather than a
    # recipe that dispatched to another recipe, so it is read from the plan.
    assert "assets.preflight" in _planned("candidate")

    # Both image stages, per architecture. The stage list is config now, so
    # this reads it rather than repeating it.
    assert config.imagebuild.lane_templates == ("kernel", "rootfs")
    assert "for stage in self._config.imagebuild.lane_templates" in lanes

    # `current` is repointed by whichever lane finished last, and the
    # host-architecture VM proof that follows needs it aimed at this machine.
    # Through the filesystem primitive, so the operation is one the harness
    # owns rather than a raw `Path.symlink_to` no dry run could show.
    assert "link(current, self.host_arch.name)" in assets
    assert "current.readlink()" in assets

    # Hash aliases before the manifest check, or startup falls through to a
    # remote fetch for a local-only asset version and the gate stops being
    # hermetic.
    assert assets.index("hash_assets_script") < assets.index('"manifest", "check"')

    # AF_UNIX paths must stay under macOS SUN_LEN once the gateway appends its
    # session path, so the run dir lives outside the descriptive scratch root.
    assert config.assets.run_dir_template.startswith("/tmp/")
    # Through the filesystem primitive, whose `parent` is required precisely
    # so a scratch dir cannot land in $TMPDIR and blow the socket limit.
    assert "scratch_dir(" in assets

    assert "shell_proof_script" in assets
    assert config.assets.shell_proof_script.endswith("prove-installed-shell.py")


def test_a_failed_boot_preserves_only_host_side_evidence() -> None:
    """A blanket copy also takes the guest's workspace into cache/target/.

    The snapshots duplicate that workspace once per generation, and the same
    name filter is what keeps the VM disk image and session.db out.
    """
    from capsem_builder.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)

    assert set(config.assets.evidence_prune_dirs) == {"guest"}
    assert ".log" in config.assets.evidence_suffixes
    assert ".toml" in config.assets.evidence_suffixes, (
        "vm/active_policy.toml records the policy a boot failure is argued from"
    )


def test_asset_gate_runs_architecture_lanes_in_parallel_before_boot_proofs() -> None:
    """Both lanes complete before anything merges or boots.

    A hosted release runner has an observed hard lifetime below the workflow's
    nominal timeout, so the four-cell matrix only fits if the architectures
    build concurrently -- and merging before both finish would publish a
    manifest for assets that do not exist yet.
    """
    assets = _source_text("build_system/builder/gate/assets.py")
    lanes = _source_text("build_system/builder/gate/assetlanes.py")

    assert "ThreadPoolExecutor" in lanes
    # The lanes are plan steps now, not a call inside this module: `sweep`
    # depends on both and `assemble` on `sweep`, which is the same ordering
    # expressed where the graph can enforce it rather than where only reading
    # the source could reveal it.
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_labels

    labels = list(gate_labels("candidate"))
    for arch in ("arm64", "x86_64"):
        assert labels.index(f"assets.build.{arch}") < labels.index("assets.sweep")
    assert labels.index("assets.sweep") < labels.index("assets.pack-initrds")
    assert labels.index("assets.pack-initrds") < labels.index("assets.assemble")
    assert assets.index("self._merge_lanes(") < assets.index("self._prove(")


def test_asset_gate_reaps_gateway_and_service_after_the_boot_proof() -> None:
    """The proof's daemons stop before anything after it starts its own.

    The gateway goes first: it owns the fixed localhost port, and one that
    outlives its service attaches the next run to a UDS pointing at a run
    directory that has already been deleted.
    """
    from capsem_builder.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    assets = _source_text("build_system/builder/gate/assets.py")

    assert config.pidfiles.names == ("gateway.pid", "service.pid")
    assert "pidfiles.stop_gate_service" in assets
    # In the `finally`, so an aborted proof still reaps -- and the reap comes
    # before the scratch is discarded, because discarding it is what destroys
    # the serial log the reap flushes.
    finally_at = assets.index("finally:", assets.index("def _prove("))
    assert finally_at < assets.index("discard(run_dir)")
    assert assets.index("pidfiles.stop_gate_service", finally_at) < assets.index("discard(run_dir)")


def test_asset_ci_uses_primitives_owned_by_just_test() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    lanes = _source_text("build_system/builder/gate/assetlanes.py")

    assert "ASSET_ARCH: ${{ matrix.arch }}" in workflow
    # The recipe takes the architecture alone.
    assert re.search(r'^\s*just build-assets "\$ASSET_ARCH"$', workflow, re.MULTILINE)
    assert "pack-initrds" in _planned("build-assets", arch="arm64", template="rootfs")
    # The lanes reach the same builder invocation directly, each with its own
    # output. Asserting they still *mention* the retired recipe was asserting
    # on a comment; this is the claim underneath it.
    assert "imagebuild.build_argv(" in lanes
    assert "output=str(output)" in lanes


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
    assert build_assets.index(pnpm_setup) < build_assets.index("just build-assets")


def test_asset_matrix_preflights_once_and_reuses_the_public_build_primitive() -> None:
    """One builder invocation, reused per stage, preflighted once.

    The lanes used to reach it through `just _build-image-template`, so this
    asserted the dispatch text. They call `build_argv` directly now -- the same
    single spelling, without a second gate process between the lane and the
    builder.
    """
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.imagebuild import build_argv

    config = gate_config.load(PROJECT_ROOT)
    lanes = (PROJECT_ROOT / "build_system/builder/gate/assetlanes.py").read_text(encoding="utf-8")

    # Every stage the lanes build goes through the one primitive.
    assert "imagebuild.build_argv(" in lanes
    for stage in config.imagebuild.lane_templates:
        argv = build_argv(config, arch="arm64", template=stage)
        assert argv[argv.index("--template") + 1] == stage

    # The primitive builds; it does not preflight. Preflighting per stage is
    # what made a four-cell matrix run doctor four times.
    assert "install-tools" not in " ".join(build_argv(config, arch="arm64", template="all"))
    assert "doctor" not in lanes


def test_in_container_commands_write_only_where_the_container_user_owns() -> None:
    """/src is a bind mount of the host checkout. On Linux the host UID does not
    own it, so anything `docker exec -u capsem` writes outside an explicitly
    chowned path fails with EACCES -- and macOS maps the mount cleanly, so only
    CI ever sees it. Four separate release-gate failures came from this one
    shape: the builder's git, the staging rm, pytest's cache, and an
    unmaterialized generated tree."""
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.docker import Docker
    from capsem_builder.gate.installcontainer import claim_owned_paths
    from helpers.gate import RecordingRunner

    config = gate_config.load(PROJECT_ROOT)
    guest = config.install.guest_user
    container = (
        PROJECT_ROOT / "build_system" / "builder" / "gate" / "installcontainer.py"
    ).read_text()
    proof = (PROJECT_ROOT / "build_system" / "builder" / "gate" / "installproof.py").read_text()

    # Replacing any owned path needs write permission on its parent. Parents
    # are derived from the typed layout and claimed without recursively
    # walking unrelated Cargo or release output.
    assert "settings.layout.owned_parent_paths(settings.mount)" in container
    assert "claim_owned_paths(self._docker, self.name, self._settings)" in container
    runner = RecordingRunner(PROJECT_ROOT)
    claim_owned_paths(Docker(runner), config.install.container, config.install)
    assert list(runner.commands[-1].argv) == [
        "docker",
        "exec",
        config.install.container,
        "chown",
        f"{guest.name}:{guest.name}",
        *config.install.layout.owned_parent_paths(config.install.mount),
    ]
    assert '["chown", "-R", f"{guest}:{guest}", *parents]' not in container

    # Every path this user writes has to live off the bind mount.
    for path in (guest.tmp, guest.pytest_cache, guest.asset_manifest, config.install.venv):
        assert path.startswith(guest.home), (
            f"{path} is not under the container user's home, so it may land on "
            "the bind mount and fail with EACCES on Linux"
        )
        assert not path.startswith(config.install.mount)

    assert "TMPDIR" in proof and "guest.tmp" in proof
    assert "cache_dir=" in proof and "pytest_cache" in proof


def test_runtime_recipes_prepare_the_signed_runtime_before_service() -> None:
    # Runtime preparation is one graph now: the guest runtime is packed before
    # host compilation/signing, and the service cannot prepare until that
    # exact signed runtime exists.
    for command in ("ensure-service", "shell", "exec"):
        plan = _command(command, guest_command="true")._describe()
        assert plan.after_of("prepare.build-binaries") == {"initrd.hash-aliases"}
        assert plan.after_of("prepare.sign") == {"prepare.build-binaries"}
        assert plan.after_of("prepare") == {"prepare.sign"}


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

    from capsem_builder.gate import cli  # noqa: F401 - registers every command
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.command import GateCommand
    from helpers.gate import RecordingRunner

    workspace_source = (PROJECT_ROOT / "build_system/builder/gate/workspace.py").read_text(
        encoding="utf-8"
    )
    pidfile_source = (PROJECT_ROOT / "build_system/builder/gate/pidfiles.py").read_text(
        encoding="utf-8"
    )

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


def _workflow_step(workflow: str, name: str) -> str:
    """One named step's body, up to the next step of the same job."""
    body = workflow.split(f"- name: {name}", maxsplit=1)[1]
    return body.split("\n      - ", maxsplit=1)[0]


def test_asset_workflow_publishes_obom_not_debug_build_ledger() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    release = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    stager = (
        PROJECT_ROOT / "build_system/builder/release/tools/stage_runtime_publication.py"
    ).read_text()

    assert "npm install -g @cyclonedx/cdxgen" not in workflow
    assert "CAPSEM_CDXGEN_CMD" not in workflow
    stage_step = _workflow_step(workflow, "Stage and verify immutable runtime publication once")
    upload_step = _workflow_step(workflow, "Publish immutable GitHub runtime release")
    attest_step = _workflow_step(workflow, "Attest VM asset provenance")
    assert "build_system/scripts/release/stage-runtime-publication.py" in stage_step
    assert "build_system/scripts/release/verify-runtime-publication.py" in stage_step
    assert "build_system/scripts/release/stage-runtime-publication.py" not in upload_step
    assert "build_system/scripts/release/verify-runtime-publication.py" in upload_step
    assert "build_system/scripts/release/publish-immutable-release-assets.sh" in upload_step
    assert "gh release create" not in upload_step
    assert "gh release upload" not in upload_step
    assert 'files=("$RELEASE_DIR"/*)' in upload_step
    assert 'for section in ("images", "evidence")' in stager
    assert "subject-path: cache/target/asset-release/runtime-*/*" in attest_step
    assert "vm-build-ledger-" not in workflow
    assert "build-ledger.log" not in upload_step
    assert "build-ledger.log" not in attest_step
    assert "B3SUMS" not in upload_step
    assert "B3SUMS" not in attest_step
    assert "obom.cdx.json" not in release
    assert "Skipping debug-only $arch/$base from release upload" not in release
