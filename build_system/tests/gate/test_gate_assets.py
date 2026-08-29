"""Everything either side of the asset build: preflight, merge, and boot proof.

Two details here are load-bearing and neither is obvious from reading the
steps. `current` is a symlink the image builders repoint at whichever
architecture they built last, so the merged manifest generator leaves it aimed
wherever the final lane finished -- and the host-architecture VM proof that
follows needs it aimed at this machine. And the installed runtime resolves
content-addressed filenames while build output uses canonical logical names, so
without the hash aliases the gate quietly falls through to a remote fetch and
stops being hermetic.
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import imagebases
from capsem_builder.gate.assets import AssetGate
from capsem_builder.gate.errors import GateError
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)


def _run_all(gate) -> None:
    """Every phase of the asset gate, in the order the plan composes them.

    `run()` did all of this behind one `Call`, with the two architecture lanes
    on a thread pool the graph could not see. They are steps now, so a test
    that wants the whole phase says so -- and one that wants a single lane can
    finally ask for it.
    """
    gate.prefetch()
    gate.preflight()
    imagebases.materialize_rust_builders(gate._runner, gate._config)
    for name in gate._config.architectures:
        gate.lane(name)
    gate.sweep()
    gate.assemble()


def _checkout(tmp_path: Path, *, profiles: tuple[str, ...] = ("code",)) -> Path:
    (tmp_path / "config").mkdir(parents=True)
    gate = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    (tmp_path / "config" / "gate.toml").write_text(
        gate.replace('parent = "~/.cg"', f'parent = "{tmp_path / "prefixes"}"')
    )
    image_config = tmp_path / "config" / "docker" / "image"
    image_config.mkdir(parents=True)
    shutil.copy2(PROJECT_ROOT / "config" / "docker" / "image" / "build.toml", image_config)
    build = imagebases.build_config(gate_config.load(tmp_path))
    for relative in (
        build.guest_rust_builder.dockerfile,
        *build.guest_rust_builder.identity_inputs,
    ):
        destination = tmp_path / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(PROJECT_ROOT / relative, destination)
    for name in profiles:
        directory = tmp_path / "config" / "profiles" / name
        directory.mkdir(parents=True)
        (directory / "profile.toml").write_text(f'id = "{name}"\n')
    for relative in CONFIG.assets.identity_roots:
        source = PROJECT_ROOT / relative
        destination = tmp_path / relative
        if destination.exists():
            continue
        if source.is_dir():
            destination.mkdir(parents=True)
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(f"fixture for {relative}\n", encoding="utf-8")
    return tmp_path


def _obom_payload(arch: str) -> str:
    return json.dumps(
        {
            "bomFormat": "CycloneDX",
            "metadata": {
                "tools": {"components": [{"name": "cdxgen", "version": "12.7.0"}]},
                "component": {
                    "type": "operating-system",
                    "name": f"capsem-rootfs-{arch}",
                    "version": "guest-rootfs",
                    "properties": [
                        {"name": "capsem:evidence:scope", "value": "exported-rootfs"},
                        {"name": "capsem:guest:architecture", "value": arch},
                    ],
                },
            },
            "components": [{"purl": "pkg:deb/debian/base-files@1"}],
        }
    )


class Gating(RecordingRunner):
    """A runner whose lanes produce artifacts and whose proofs succeed."""

    def execute(self, command):
        completed = super().execute(command)
        rendered = str(command)
        # Keyed on the builder's own argv: the lane no longer goes through a
        # dispatcher, and the output root it names is the thing under test.
        if "--output" in command.argv and "--arch" in command.argv:
            output = command.argv[command.argv.index("--output") + 1]
            arch = command.argv[command.argv.index("--arch") + 1]
            produced = Path(output) / arch
            produced.mkdir(parents=True, exist_ok=True)
            config = gate_config.for_root(self.root)
            for name in (*config.artifacts.bootable, *config.assets.evidence_artifacts):
                payload = _obom_payload(arch) if name == config.assets.obom_artifact else "bytes"
                (produced / name).write_text(payload)
        if "manifest generate" in rendered:
            Path(command.argv[-1]).mkdir(parents=True, exist_ok=True)
            (Path(command.argv[-1]) / "manifest.json").write_text("{}")
        if "profile materialize" in rendered:
            output = Path(command.argv[command.argv.index("--output-root") + 1])
            profile = Path(command.argv[command.argv.index("--profile") + 1]).parent.name
            materialized = output / "profiles" / profile
            materialized.mkdir(parents=True, exist_ok=True)
            (materialized / "profile.toml").write_text(f'id = "{profile}"\n')
            config = gate_config.for_root(self.root)
            manifest = output / config.suites.pytest.test_manifest
            manifest.parent.mkdir(parents=True, exist_ok=True)
            manifest.write_text("{}")
        return completed


def _gate(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    *,
    runner_class: type[Gating] = Gating,
    profiles: tuple[str, ...] = ("code",),
    **kwargs,
) -> tuple[AssetGate, Gating]:
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem_builder.gate.pidfiles.stop_gate_service", lambda *_a: None)
    monkeypatch.setattr("capsem_builder.gate.assets.WaitForSocket.perform", lambda _self, _context: None)
    runner = runner_class(_checkout(tmp_path, profiles=profiles), **kwargs)
    return AssetGate(runner), runner


# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------


def test_cross_architecture_execution_is_proven_before_any_lane_starts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Discovering Docker cannot run the other architecture an hour in wastes
    the whole matrix."""
    gate, runner = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    runner.assert_order(r"docker run --rm --network none --platform", r"image build")


def test_cross_architecture_execution_is_proven_before_rust_builder_prewarm(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Do not spend a cold Cargo prewarm before proving the target can execute."""
    gate, runner = _gate(
        tmp_path,
        monkeypatch,
        failures=("docker image inspect capsem-guest",),
    )

    _run_all(gate)

    runner.assert_order(
        r"docker run --rm --network none --platform",
        r"docker build .*Dockerfile\.guest-rust-builder",
    )


def test_cold_exact_base_images_are_pulled_before_the_cross_architecture_probe(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, runner = _gate(tmp_path, monkeypatch, failures=["docker image inspect"])

    gate.prefetch()
    gate.preflight()

    for arch in gate.build_config.architectures.values():
        pulled = f"docker pull --platform {arch.docker_platform} {arch.base_image}"
        assert runner.matching(pulled)
        runner.assert_order(pulled, r"docker run --rm --network none --platform")


def test_warm_exact_base_images_do_not_contact_the_registry(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, runner = _gate(tmp_path, monkeypatch)

    gate.prefetch()

    assert not runner.matching(r"docker pull")


def test_the_probe_targets_the_architecture_this_host_is_not(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, runner = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    probe = runner.matching(r"docker run --rm --network none --platform")[0]
    assert f"linux/{gate.host_arch.dpkg}" not in probe
    other = next(
        arch
        for name, arch in gate.build_config.architectures.items()
        if name != gate.host_arch.name
    )
    assert other.base_image in probe
    # And it declares what it needs. The probe runs `/bin/true` to ask whether
    # the daemon can execute the other architecture; it needs nothing from the
    # network, and the wrapper requires every container to say so rather than
    # getting outbound access by omission.
    assert "--network none" in probe


def test_a_host_that_cannot_run_the_other_architecture_says_how_to_fix_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, _ = _gate(tmp_path, monkeypatch, failures=["docker run"])

    with pytest.raises(GateError) as failure:
        _run_all(gate)

    assert "colima restart" in str(failure.value), "macOS needs its own remedy"


def test_a_linux_host_is_told_about_binfmt_instead(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, _ = _gate(tmp_path, monkeypatch, failures=["docker run"])
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")

    with pytest.raises(GateError, match="binfmt QEMU"):
        _run_all(gate)


# ---------------------------------------------------------------------------
# Assembly
# ---------------------------------------------------------------------------


def test_both_architectures_are_merged_into_one_asset_tree(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, _ = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    assets = gate.test_root / "code" / "assets"
    for arch in CONFIG.architectures:
        assert (assets / arch).is_dir(), f"{arch} never reached the merged tree"


def test_current_is_restored_to_the_host_architecture_after_generation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The builders repoint `current` at whichever architecture they built
    last, and the VM proof that follows runs on this machine."""
    gate, runner = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    current = gate.test_root / "code" / "assets" / "current"
    assert current.readlink().name == gate.host_arch.name
    runner.assert_order(r"manifest generate", r"create_hash_assets\.py")


def test_hash_aliases_are_materialized_before_the_manifest_is_checked(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Without them, startup falls through to a remote fetch for a local-only
    asset version and the gate is no longer hermetic."""
    gate, runner = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    runner.assert_order(r"create_hash_assets\.py", r"manifest check")


def test_every_runtime_profile_is_materialized_against_the_generated_manifest(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, runner = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    materialized = runner.matching(r"profile materialize")
    assert materialized
    assert all("file://" in line for line in materialized), (
        "the profile must be materialized against the manifest just generated, "
        "not against a channel URL"
    )


def test_the_boot_proof_runs_after_the_profile_is_materialized(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, runner = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    runner.assert_order(r"profile materialize", r"prove-installed-shell\.py")


def test_the_boot_service_is_detached_owned_and_ready_before_the_shell_proof(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A foreground proof may not leave its auto-started daemon behind.

    The process-tree guard caught exactly that in a retained candidate: the
    successful shell script exited while its descendant ``capsem-service``
    remained alive.  The gate must own the daemon through the launch/pidfile
    lifecycle before the foreground proof starts.
    """
    gate, runner = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    launched = runner.matching(r"capsem-service .*--foreground")
    assert len(launched) == 1
    assert f"--assets-dir {gate.test_root}/code/assets" in launched[0]
    assert f"CAPSEM_PROFILES_DIR={gate.test_root}/code/config/profiles" in launched[0]
    runner.assert_order(r"capsem-service .*--foreground", r"prove-installed-shell\.py")


def test_the_boot_proof_runs_against_this_profile_s_own_assets(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A proof reading the shared assets tree would pass for a profile whose
    own build was broken."""
    gate, runner = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    proof = runner.matching(r"prove-installed-shell\.py")[0]
    assert f"CAPSEM_ASSETS_DIR={gate.test_root}/code/assets" in proof
    assert "CAPSEM_RUN_DIR=/" in proof


def test_verified_base_profile_becomes_the_canonical_following_input(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Build-chain, packaging and glow-up must consume the bytes IronBank booted.

    A warm canonical tree used to survive the private profile build. IronBank
    proved ``target/ironbank-assets/code`` and the following modules silently
    opened the older ``assets/`` and ``target/config/profiles`` instead.
    """
    gate, _ = _gate(tmp_path, monkeypatch, profiles=("co-work", "code"))
    root = gate.root
    stale_assets = root / CONFIG.functional.assets_dir
    stale_assets.mkdir(parents=True)
    (stale_assets / "manifest.json").write_text('{"stale":true}\n')
    stale_profiles = root / CONFIG.functional.config_root / CONFIG.functional.profiles_subdir
    stale_profiles.mkdir(parents=True)
    (stale_profiles / "stale.toml").write_text("stale = true\n")
    stale_config = root / CONFIG.functional.config_root
    stale_config_manifest = stale_config / CONFIG.suites.pytest.test_manifest
    stale_config_manifest.parent.mkdir(parents=True, exist_ok=True)
    stale_config_manifest.write_text('{"stale":true}\n')
    stale_sibling = stale_config / "retired" / "stale.toml"
    stale_sibling.parent.mkdir(parents=True)
    stale_sibling.write_text("stale = true\n")
    _run_all(gate)

    selected_assets = gate.test_root / CONFIG.suites.pytest.base_profile / "assets"
    assert stale_assets.is_symlink()
    assert stale_assets.resolve() == selected_assets.resolve()
    assert (stale_assets / "manifest.json").read_text() == "{}"
    assert not (stale_profiles / "stale.toml").exists()
    assert stale_config_manifest.read_text() == "{}"
    assert not stale_sibling.exists()
    assert sorted(path.parent.name for path in stale_profiles.glob("*/profile.toml")) == [
        "co-work",
        "code",
    ]


# ---------------------------------------------------------------------------
# Failure
# ---------------------------------------------------------------------------


def test_a_failed_boot_preserves_its_evidence(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A failed proof leaves its session in place, and process.log and
    serial.log are what the failure is argued from."""

    class LeavesEvidence(Gating):
        """A boot that fails after writing the diagnostics it failed with."""

        def execute(self, command):
            if "prove-installed-shell" in str(command):
                run_dir = Path(command.env["CAPSEM_RUN_DIR"])
                (run_dir / "vm").mkdir(parents=True, exist_ok=True)
                (run_dir / "serial.log").write_text("boot failed")
                (run_dir / "vm" / "active_profile.toml").write_text("pins")
                (run_dir / "guest").mkdir(exist_ok=True)
                (run_dir / "guest" / "workspace.log").write_text("noise")
            return super().execute(command)

    gate, _ = _gate(
        tmp_path,
        monkeypatch,
        runner_class=LeavesEvidence,
        failures=["prove-installed-shell"],
    )

    with pytest.raises(GateError):
        _run_all(gate)

    preserved = gate.test_root / "code" / "run-failure"
    assert (preserved / "serial.log").is_file()
    assert (preserved / "vm" / "active_profile.toml").is_file()
    assert not (preserved / "guest").exists(), (
        "guest/ duplicates the guest's own workspace once per generation and "
        "must not be copied into target/"
    )


def test_the_run_directory_is_removed_even_when_the_proof_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    seen: list[Path] = []

    class Watching(Gating):
        def execute(self, command):
            if "prove-installed-shell" in str(command):
                seen.append(Path(command.env["CAPSEM_RUN_DIR"]))
            return super().execute(command)

    gate, _ = _gate(
        tmp_path, monkeypatch, runner_class=Watching, failures=["prove-installed-shell"]
    )

    with pytest.raises(GateError):
        _run_all(gate)

    assert seen and not seen[0].exists()


def test_leftover_containers_are_cleaned_up_after_the_lanes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A build container holding the scratch tree open outlives its lane."""
    gate, runner = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    runner.assert_order(r"image build", r"cleanup-docker-containers-by-mount\.sh")
