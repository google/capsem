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
from capsem.gate.crosscompile import (
    PackageRail,
    pinned_toolchain,
    resolve_channel,
    signing_key,
)
from capsem.gate.errors import GateError

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
    (tmp_path / "rust-toolchain.toml").write_text(
        f'[toolchain]\nchannel = "{toolchain}"\n'
    )
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
    (tmp_path / "rust-toolchain.toml").write_text("[other]\n")

    with pytest.raises(GateError, match=r"no .toolchain. channel"):
        pinned_toolchain(tmp_path)


def test_release_keys_are_used_when_the_checkout_has_them(tmp_path: Path) -> None:
    (tmp_path / "private" / "tauri").mkdir(parents=True)
    (tmp_path / "private" / "tauri" / "capsem.key").write_text("KEY")
    (tmp_path / "private" / "tauri" / "password.txt").write_text("PASS")

    assert signing_key(tmp_path) == {
        "TAURI_SIGNING_PRIVATE_KEY": "KEY",
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": "PASS",
    }


def test_a_checkout_without_keys_injects_none(tmp_path: Path) -> None:
    """The container then makes a throwaway dev key. The authoritative keys
    live in Actions secrets and are applied only on publish."""
    (tmp_path / "private" / "tauri").mkdir(parents=True)
    (tmp_path / "private" / "tauri" / "capsem.key").write_text("KEY")

    assert signing_key(tmp_path) == {}


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

    _rail(runner).run()

    build = runner.matching(r"docker run --rm")[0]
    assert f"TARGET_ARCH={TARGET.name}" in build
    assert f"RUST_TARGET={TARGET.rust_target}" in build
    assert f"DPKG_ARCH={TARGET.dpkg}" in build
    assert "RUST_TOOLCHAIN=1.2.3" in build
    assert f"PKG_CONFIG_PATH={TARGET.pkg_config_path}" in build
    assert f"bash /src/{BUILD_SCRIPT}" in build


def test_the_cargo_caches_are_shared_and_the_target_dir_is_per_architecture(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A shared /cargo-target across architectures would rebuild the world on
    every alternation; a per-architecture registry would refetch the index."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _rail(runner).run()

    build = runner.matching(r"docker run --rm")[0]
    assert "-v capsem-cargo-registry:/usr/local/cargo/registry" in build
    assert "-v capsem-rustup:/usr/local/rustup" in build
    assert f"-v capsem-host-target-{TARGET.name}:/cargo-target" in build


def test_the_builder_image_is_rebuilt_before_every_package(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _rail(runner).run()

    runner.assert_order(r"just _build-host-image", r"docker run --rm")


def test_the_container_clock_is_synced_only_on_macos(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Colima's VM clock drifts and apt rejects a repository signed in what it
    believes is the future. A Linux runner has no such VM."""
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    for system, expected in (("Darwin", True), ("Linux", False)):
        monkeypatch.setattr("capsem.gate.host.system", lambda system=system: system)
        runner = Building(
            _checkout(tmp_path / system), replies={"select-linux": "skip"}
        )

        _rail(runner).run()

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

    assert _rail(runner).run() == root / "dist" / PACKAGE


def test_a_build_that_recorded_nothing_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), records=None)

    with pytest.raises(GateError, match="did not record the exact Debian package"):
        _rail(runner).run()


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
        _rail(runner).run()


def test_the_record_does_not_survive_the_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Left behind, it would name this run's package to the next one."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    root = _checkout(tmp_path)
    runner = Building(root, replies={"select-linux": "skip"})

    _rail(runner).run()

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

    _rail(runner, channel="nightly", manifest_url="file:///src/m.json").run()

    proof = runner.matching(r"just _prove-linux-deb")[0]
    assert "CAPSEM_PROOF_MANIFEST_CHANNEL=nightly" in proof
    assert "CAPSEM_PROOF_MANIFEST_URL=file:///src/m.json" in proof
    assert PACKAGE in proof


def test_a_cross_target_skips_the_proof_and_says_why(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The decision belongs to `select-linux-deb-proof.sh`; this must not
    second-guess it, or the two disagree about what a green run proved."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _rail(runner).run()

    assert not runner.ran(r"just _prove-linux-deb")
    assert any("Skipping exact Debian package proof" in note for note in runner.notes)
