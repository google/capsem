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

from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate.assets import AssetGate
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)



def _run_all(gate) -> None:
    """Every phase of the asset gate, in the order the plan composes them.

    `run()` did all of this behind one `Call`, with the two architecture lanes
    on a thread pool the graph could not see. They are steps now, so a test
    that wants the whole phase says so -- and one that wants a single lane can
    finally ask for it.
    """
    gate.preflight()
    for name in gate._config.architectures:
        gate.lane(name)
    gate.sweep()
    gate.assemble()


def _checkout(tmp_path: Path, *, profiles: tuple[str, ...] = ("code",)) -> Path:
    (tmp_path / "config").mkdir(parents=True)
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    for name in profiles:
        directory = tmp_path / "config" / "profiles" / name
        directory.mkdir(parents=True)
        (directory / "profile.toml").write_text(f'id = "{name}"\n')
    return tmp_path


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
                (produced / name).write_text("bytes")
        if "manifest generate" in rendered:
            Path(command.argv[-1]).mkdir(parents=True, exist_ok=True)
            (Path(command.argv[-1]) / "manifest.json").write_text("{}")
        return completed


def _gate(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    *,
    runner_class: type[Gating] = Gating,
    **kwargs,
) -> tuple[AssetGate, Gating]:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem.gate.pidfiles.stop_gate_service", lambda *_a: None)
    runner = runner_class(_checkout(tmp_path), **kwargs)
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

    runner.assert_order(r"docker run --rm --platform", r"image build")


def test_the_probe_targets_the_architecture_this_host_is_not(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, runner = _gate(tmp_path, monkeypatch)

    _run_all(gate)

    probe = runner.matching(r"docker run --rm --platform")[0]
    assert f"linux/{gate.host_arch.dpkg}" not in probe


def test_a_host_that_cannot_run_the_other_architecture_says_how_to_fix_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, _ = _gate(tmp_path, monkeypatch, failures=["--platform"])

    with pytest.raises(GateError) as failure:
        _run_all(gate)

    assert "colima restart" in str(failure.value), "macOS needs its own remedy"


def test_a_linux_host_is_told_about_binfmt_instead(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    gate, _ = _gate(tmp_path, monkeypatch, failures=["--platform"])
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")

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
