"""The package rail builds one architecture, and proves which package it built.

The rail's sharpest rule is that it publishes the package *this run* produced.
`dist/` accumulates, so globbing it would let a package built from a different
commit be proved, installed, and shipped -- which is why the builder writes the
basename it created and this reads it back rather than looking around.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate.dockermount import Mount
from capsem.gate.errors import GateError
from capsem.gate.packageinputs import pinned_toolchain, resolve_channel
from capsem.gate.packagerail import PackageRail
from capsem.gate.packagesigning import signing_key

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
BUILD_SCRIPT = CONFIG.package.build_script
TARGET = CONFIG.arch("arm64")
PACKAGE = "Capsem_9.9.9_arm64.deb"


def _checkout(tmp_path: Path, *, toolchain: str = "9.99.9") -> Path:
    """A fake checkout carrying the real gate configuration.

    The rail reads `config/gate.toml` for volume names and scripts, so the
    fixture links it rather than inventing a second copy that could drift from
    the one the gate actually runs with.
    """
    tmp_path.mkdir(parents=True, exist_ok=True)
    (tmp_path / CONFIG.package.toolchain_pin).write_text(f'[toolchain]\nchannel = "{toolchain}"\n')
    (tmp_path / "scripts").mkdir()
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    (tmp_path / "assets" / TARGET.name).mkdir(parents=True)
    return tmp_path


class Building(RecordingRunner):
    """A runner whose builder container writes the package record."""

    def __init__(self, root: Path, *, records: str | None = PACKAGE, **kwargs) -> None:
        super().__init__(root, **kwargs)
        self._records = records

    def execute(self, command):
        completed = super().execute(command)
        if BUILD_SCRIPT in str(command) and self._records is not None:
            (self.root / "dist").mkdir(exist_ok=True)
            (self.root / "dist" / f".cross-compile-{TARGET.name}-deb").write_text(
                self._records + "\n"
            )
            target = self.root / "dist" / self._records
            if target.parent == self.root / "dist":
                target.write_text("package bytes")
        return completed


def _run_lane(rail):
    """Every phase of one lane, in the order the plan composes them.

    The rail used to have a `run()` that did all of this behind one `Call`.
    The phases are plan steps now, so a test that wants the whole lane says
    so -- and a test that wants one phase can finally ask for one.
    """
    rail.release_rails()
    rail.reserve()
    rail.sync_clock()
    rail.sync_assets()
    rail.build()
    package = rail.resolve()
    rail.prove()
    rail.collect()
    return package


def _rail(runner: RecordingRunner, **kwargs) -> PackageRail:
    return PackageRail(runner, TARGET, **kwargs)


# ---------------------------------------------------------------------------
# Inputs read rather than repeated
# ---------------------------------------------------------------------------


def test_the_toolchain_comes_from_the_file_that_pins_it(tmp_path: Path) -> None:
    """It was spelled three times inside one inline shell script -- three
    chances for a toolchain bump to leave the package rail behind."""
    root = _checkout(tmp_path, toolchain="1.2.3")

    assert pinned_toolchain(root) == "1.2.3"


def test_a_checkout_with_no_pinned_toolchain_says_so(tmp_path: Path) -> None:
    root = _checkout(tmp_path)
    (root / CONFIG.package.toolchain_pin).write_text("[other]\n")

    with pytest.raises(GateError, match=r"no .toolchain. channel"):
        pinned_toolchain(root)


def test_release_keys_are_used_when_the_checkout_has_them(tmp_path: Path) -> None:
    (tmp_path / "private" / "tauri").mkdir(parents=True)
    (tmp_path / "private" / "tauri" / "capsem.key").write_text("KEY")
    (tmp_path / "private" / "tauri" / "password.txt").write_text("PASS")

    assert signing_key(tmp_path, CONFIG) == {
        "TAURI_SIGNING_PRIVATE_KEY": "KEY",
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": "PASS",
    }


def test_a_checkout_without_keys_injects_none(tmp_path: Path) -> None:
    """The container then makes a throwaway dev key. The authoritative keys
    live in Actions secrets and are applied only on publish."""
    (tmp_path / "private" / "tauri").mkdir(parents=True)
    (tmp_path / "private" / "tauri" / "capsem.key").write_text("KEY")

    assert signing_key(tmp_path, CONFIG) == {}


@pytest.mark.parametrize("channel", CONFIG.package.channels)
def test_known_channels_are_accepted(channel: str) -> None:
    assert resolve_channel(channel, CONFIG) == channel


def test_an_unknown_channel_is_refused_before_anything_is_built() -> None:
    with pytest.raises(GateError, match="stable, nightly, corp"):
        resolve_channel("prod", CONFIG)


# ---------------------------------------------------------------------------
# The build
# ---------------------------------------------------------------------------


def test_the_builder_receives_every_name_for_the_target(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path, toolchain="1.2.3"), replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    build = runner.matching(r"docker create")[0]
    # Forwarded by name, never as `NAME=value`: the same argv carries the
    # Tauri signing key, and a value in argv is world-readable through `ps`.
    # The values are asserted on the recorded environment instead.
    for name in ("TARGET_ARCH", "RUST_TARGET", "DPKG_ARCH", "RUST_TOOLCHAIN", "PKG_CONFIG_PATH"):
        assert f"-e {name}" in build, f"{name} is not handed to the builder"

    created = next(c for c in runner.commands if c.argv[:2] == ("docker", "create"))
    assert created.env["TARGET_ARCH"] == TARGET.name
    assert created.env["RUST_TARGET"] == TARGET.rust_target
    assert created.env["DPKG_ARCH"] == TARGET.dpkg
    assert created.env["RUST_TOOLCHAIN"] == "1.2.3"
    assert created.env["PKG_CONFIG_PATH"] == TARGET.pkg_config_path
    assert f"bash /src/{BUILD_SCRIPT}" in build


def test_the_cargo_caches_are_shared_and_the_target_dir_is_per_architecture(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A shared /cargo-target across architectures would rebuild the world on
    every alternation; a per-architecture registry would refetch the index."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    build = runner.matching(r"docker create")[0]
    assert "-v capsem-cargo-registry:/usr/local/cargo/registry" in build
    assert "-v capsem-rustup:/usr/local/rustup" in build
    assert f"-v capsem-host-target-{TARGET.name}:/cargo-target" in build


def test_package_build_mounts_linked_worktree_git_metadata(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    metadata = "/git/common"
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    monkeypatch.setattr(
        "capsem.gate.packagerail.docker_git_metadata_mount",
        lambda _runner: Mount.unmigrated(metadata, metadata, "ro"),
    )
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    build = runner.matching(r"docker create")[0]
    assert f"-v {metadata}:{metadata}:ro" in build


def test_the_builder_image_is_rebuilt_before_every_package() -> None:
    """Always rebuilt, and always before the package that runs inside it.

    The claim is unchanged; the evidence moved. The rail used to run `just
    _build-host-image` itself, and this asserted the ordering by watching the
    runner. That recipe never existed -- it has a heading in the justfile and
    no body -- so what this actually proved was that the rail issued a command
    which failed. Watching a runner cannot tell those apart.

    The image is a step now, and the order is an edge, so the assertion is
    about the graph rather than about a sequence of attempts.
    """
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import (
        cli,  # noqa: F401 - registers every command
        hostimage,
    )
    from capsem.gate.command import GateCommand

    plan = GateCommand.registry["cross-compile"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, arch=TARGET.name),
    )._describe()
    order = list(plan.labels)

    assert order.index(hostimage.STEP) < order.index(f"package.{TARGET.name}.build")
    # The lane's first phase depends on the image; the rest chain from there.
    # It was one step, so the edge landed on the whole lane -- which is also
    # why nothing could be ordered against a phase inside it.
    assert (hostimage.STEP, f"package.{TARGET.name}.storage-release") in plan.edges


def test_the_container_clock_is_synced_only_on_macos(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Colima's VM clock drifts and apt rejects a repository signed in what it
    believes is the future. A Linux runner has no such VM."""
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    for system, expected in (("Darwin", True), ("Linux", False)):
        monkeypatch.setattr("capsem.gate.host.system", lambda system=system: system)
        runner = Building(_checkout(tmp_path / system), replies={"select-linux": "skip"})

        _run_lane(_rail(runner))

        assert runner.ran(r"sync-container-clock\.py") is expected


# ---------------------------------------------------------------------------
# Which package got built
# ---------------------------------------------------------------------------


def test_the_recorded_package_is_the_one_this_run_produced(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    root = _checkout(tmp_path)
    # A package from an earlier build of a different commit, still in dist/.
    (root / "dist").mkdir()
    (root / "dist" / "Capsem_0.0.1_arm64.deb").write_text("stale")
    runner = Building(root, replies={"select-linux": "skip"})

    assert _run_lane(_rail(runner)) == root / "dist" / PACKAGE


def test_a_build_that_recorded_nothing_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), records=None)

    with pytest.raises(GateError, match="did not record the exact Debian package"):
        _run_lane(_rail(runner))


@pytest.mark.parametrize(
    "recorded, reason",
    [
        ("capsem.tar.gz", "invalid Debian package record"),
        ("../outside/capsem.deb", "escaped dist/"),
    ],
)
def test_a_nonsense_package_record_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, recorded: str, reason: str
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), records=recorded)

    with pytest.raises(GateError, match=reason):
        _run_lane(_rail(runner))


def test_the_record_does_not_survive_the_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Left behind, it would name this run's package to the next one."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    root = _checkout(tmp_path)
    runner = Building(root, replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    assert not (root / "dist" / f".cross-compile-{TARGET.name}-deb").exists()


# ---------------------------------------------------------------------------
# Whether the package gets proved
# ---------------------------------------------------------------------------


def test_a_provable_target_runs_the_systemd_kvm_proof(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    monkeypatch.setattr("capsem.gate.host.device_available", lambda _path: True)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "prove"})

    # The proof is called, not launched: the three `CAPSEM_PROOF_*` variables
    # existed only to carry these arguments across a process boundary that no
    # longer exists, and `DebProof` always took them as arguments. What is
    # asserted is therefore what it was handed, which is the same claim
    # without a subprocess in the middle.
    from capsem.gate import packagerail

    handed = {}

    class Recording:
        def __init__(self, _runner, **kwargs):
            handed.update(kwargs)

        def run(self):
            handed["ran"] = True

    monkeypatch.setattr(packagerail.debproof, "DebProof", Recording)

    _run_lane(_rail(runner, channel="nightly", manifest_url="file:///src/m.json"))

    assert handed.get("ran"), "a provable target did not run the proof"
    assert handed["channel"] == "nightly"
    assert handed["manifest_url"] == "file:///src/m.json"
    assert handed["package"].name == PACKAGE


def test_a_cross_target_skips_the_proof_and_says_why(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The decision belongs to `select-linux-deb-proof.sh`; this must not
    second-guess it, or the two disagree about what a green run proved."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    assert not runner.ran(r"just _prove-linux-deb")
    assert any("Skipping exact Debian package proof" in note for note in runner.notes)


def test_the_builder_environment_follows_the_configured_names() -> None:
    """A rename in config must move the rail, not silently leave it behind.

    Asserted against a *changed* config rather than the real one: a test that
    reads the same literal the implementation reads passes whether or not the
    implementation reads config at all.
    """
    from capsem.gate.packageinputs import package_environment

    renamed = CONFIG.model_copy(
        update={
            "package": CONFIG.package.model_copy(
                update={"manifest_variable": "CAPSEM_RENAMED_MANIFEST"}
            )
        }
    )
    target = CONFIG.arch(next(iter(CONFIG.architectures)))

    environment = package_environment(
        renamed,
        target,
        toolchain="1.97.1",
        manifest_url="file:///src/assets/local/manifest.json",
        signing={},
    )

    assert environment["CAPSEM_RENAMED_MANIFEST"] == ("file:///src/assets/local/manifest.json")
    assert "CAPSEM_INSTALL_MANIFEST_URL" not in environment


def test_the_builder_environment_carries_the_signing_material_it_was_given() -> None:
    from capsem.gate.packageinputs import package_environment

    target = CONFIG.arch(next(iter(CONFIG.architectures)))
    signing = {CONFIG.package.signing.key_variable: "secret-key-bytes"}

    environment = package_environment(
        CONFIG, target, toolchain="1.97.1", manifest_url="x", signing=signing
    )

    assert environment[CONFIG.package.signing.key_variable] == "secret-key-bytes"


def test_the_disk_rail_is_measured_at_two_different_moments() -> None:
    """Twice, deliberately -- but not twice in the same breath.

    The pair exists because the builder image is itself part of what fills
    this rail: one check once it exists, one immediately before the package
    build spends the headroom. Both calls sat on adjacent lines, so they
    measured the same moment and the second could only ever agree with the
    first. Removing one looked right and would have lost a real check.
    """
    source = (PROJECT_ROOT / "src" / "capsem" / "gate" / "packagerail.py").read_text(
        encoding="utf-8"
    )

    assert source.count('ensure_space("package")') == 2
    build = source.index("def build(self)")
    first = source.index('ensure_space("package")')
    second = source.index('ensure_space("package")', build)

    assert first < build < second, "both checks sit in one method, so they measure a single moment"
