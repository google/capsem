"""The install gate's order, which is the only place its defect was visible.

`capsem-admin` authors the release graph and ships inside the package under
test. The shell resolved that circle by installing first and authoring
afterwards, so the postinst ran with no manifest request to read -- and did not
fail. `capsem_resolve_install_manifest` falls back to the URL baked into the
package when the request file is absent, so the whole-world *local* proof was
hydrating from `release.capsem.org`, and reported a product failure when those
public artifacts were retired.

None of that is visible in any single command. It is visible only in the
sequence, which is why these tests assert on the sequence.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from helpers.gate import RecordingRunner

from capsem.gate import arch as architectures
from capsem.gate.errors import GateError
from capsem.gate.install import LAYOUT, InstallGate
from capsem.gate.installproof import SERVE_READY_FILE
from capsem.gate.releasegraph import GRAPH_MANIFEST, PREINSTALL_ADMIN, ReleaseGraph
from capsem.gate.docker import Docker


VERSION = "9.9.9"
WORKSPACE = f"""\
[workspace]
members = ["crates/capsem"]

[workspace.package]
version = "{VERSION}"
"""

AUTHORITATIVE = f"{LAYOUT.channel}/{GRAPH_MANIFEST}"


def _checkout(tmp_path: Path, *, dpkg_arch: str) -> Path:
    (tmp_path / "Cargo.toml").write_text(WORKSPACE)
    (tmp_path / "dist").mkdir()
    (tmp_path / "dist" / f"Capsem_{VERSION}_{dpkg_arch}.deb").write_text("package bytes")
    return tmp_path


@pytest.fixture
def gate(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple[InstallGate, RecordingRunner]:
    """A gate on a fake checkout, whose every command is recorded, not run.

    The container answers "yes" to `test -f` so staging reaches the handoff,
    reports the expected versions, and comes up as a non-Linux host -- the
    macOS shape, which is where the local proof runs.
    """
    root = _macos_checkout(tmp_path, monkeypatch)
    runner = _recording(root)
    return InstallGate(runner, macos_glowup_report=str(root / "report.json")), runner


def _macos_checkout(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A checkout on a macOS host with no colima, the local-proof shape."""
    monkeypatch.setattr("capsem.gate.arch.host_system", lambda: "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: None)
    return _checkout(tmp_path, dpkg_arch=architectures.host().dpkg)


def _recording(root: Path) -> RecordingRunner:
    return RecordingRunner(
        root,
        replies={
            "dpkg-deb -f": VERSION,
            "dpkg-query": VERSION,
            "systemctl is-system-running": "running",
        },
    )


# ---------------------------------------------------------------------------
# The ordering defect
# ---------------------------------------------------------------------------


def test_the_manifest_handoff_is_written_before_the_package_is_installed(
    gate: tuple[InstallGate, RecordingRunner],
) -> None:
    """The whole defect, in one assertion.

    Written after `dpkg -i`, the request is never read: the postinst has
    already resolved its manifest and exited.
    """
    built, runner = gate
    built.run()

    runner.assert_order(
        r"install-manifest-request\.sh write",
        r"dpkg -i",
    )


def test_the_graph_exists_before_anything_points_at_it(
    gate: tuple[InstallGate, RecordingRunner],
) -> None:
    """A handoff naming a file nobody wrote is a handoff nobody can use."""
    built, runner = gate
    built.run()

    runner.assert_order(
        r"assets channel build",
        r"assets channel check",
        r"install-manifest-request\.sh write",
    )


def test_the_admin_that_authors_the_graph_is_extracted_not_installed(
    gate: tuple[InstallGate, RecordingRunner],
) -> None:
    """The circle: the graph's author ships inside the package being installed.

    Extracting it uses the exact binary under test, and needs no install to
    have happened first.
    """
    built, runner = gate
    built.run()

    runner.assert_order(
        r"dpkg-deb --extract",
        r"assets channel record-binary",
        r"dpkg -i",
    )
    # The installed path, not merely the substring: PREINSTALL_ADMIN ends in
    # the same `/usr/bin/capsem-admin`, so a naive search matches the fix.
    assert not runner.matching(r"(?:^|[\s'\"])/usr/bin/capsem-admin"), (
        "the installed admin cannot author a graph that must exist before the "
        "install; only the extracted copy can"
    )
    assert runner.matching(PREINSTALL_ADMIN)


def test_the_handoff_is_cleared_after_the_install(
    gate: tuple[InstallGate, RecordingRunner],
) -> None:
    """A request left behind is inherited by the next install in this checkout."""
    built, runner = gate
    built.run()

    runner.assert_order(r"dpkg -i", r"install-manifest-request\.sh clear")


def test_the_proofs_run_against_the_installed_package(
    gate: tuple[InstallGate, RecordingRunner],
) -> None:
    built, runner = gate
    built.run()

    runner.assert_order(
        r"dpkg -i",
        r"pytest tests/capsem-install/",
        r"local-release-glowup\.py",
    )


def test_the_local_channel_is_served_before_the_graph_is_built(
    gate: tuple[InstallGate, RecordingRunner],
) -> None:
    built, runner = gate
    built.run()

    runner.assert_order(
        r"serve-release-test-root\.py",
        rf"test -f {SERVE_READY_FILE}",
        r"assets channel build",
    )


# ---------------------------------------------------------------------------
# What the handoff may point at
# ---------------------------------------------------------------------------


def test_the_handoff_names_the_authoritative_graph(
    gate: tuple[InstallGate, RecordingRunner],
) -> None:
    built, runner = gate
    built.run()

    written = runner.matching(r"install-manifest-request\.sh write")
    assert len(written) == 1
    assert f"write /src/{AUTHORITATIVE}" in written[0]


def test_the_legacy_runtime_projection_is_refused(tmp_path: Path) -> None:
    """Pointing at `assets/manifest.json` produces a different silent failure.

    The install succeeds and carries the legacy v2 projection instead of the
    release graph, which reads as a working install right up until something
    asks the manifest a question only the graph can answer.
    """
    runner = RecordingRunner(tmp_path)
    graph = ReleaseGraph(Docker(runner), "box")

    with pytest.raises(GateError, match="not the legacy runtime projection"):
        graph.hand_off(f"{LAYOUT.assets}/manifest.json")

    assert not runner.ran(r"install-manifest-request"), (
        "the refusal must happen before anything is written"
    )


def test_a_handoff_target_that_does_not_exist_is_refused(tmp_path: Path) -> None:
    """`install-manifest-request.sh` would refuse this too -- and then the
    postinst finds no request at all and hydrates from the public channel,
    which is exactly the silent fallback being removed."""
    runner = RecordingRunner(tmp_path, failures=["test -f"])
    graph = ReleaseGraph(Docker(runner), "box")

    with pytest.raises(GateError, match="would find no request"):
        graph.hand_off(AUTHORITATIVE)


def test_clearing_a_handoff_that_was_never_written_does_nothing(tmp_path: Path) -> None:
    """Cleanup runs on the failure path, where the write may not have happened."""
    runner = RecordingRunner(tmp_path)

    ReleaseGraph(Docker(runner), "box").clear_handoff()

    assert not runner.ran(r"install-manifest-request")


# ---------------------------------------------------------------------------
# The release-lane shape
# ---------------------------------------------------------------------------


def test_a_release_lane_stages_verified_inputs_and_hands_over_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Profile inputs a manifest already resolved carry no graph for us to publish.

    Handing over the staged v2 manifest instead is the failure above, so this
    path deliberately writes no request and lets the packaged URL apply.
    """
    root = _macos_checkout(tmp_path, monkeypatch)
    runner = _recording(root)

    InstallGate(
        runner,
        profile_inputs="target/ci-install-profile-inputs",
        macos_glowup_report=str(root / "report.json"),
    ).run()

    assert runner.matching(r"stage-release-test-inputs\.py")
    assert not runner.ran(r"install-manifest-request\.sh write")
    assert not runner.ran(r"assets channel build")
    assert runner.ran(r"dpkg -i")


# ---------------------------------------------------------------------------
# Refusals
# ---------------------------------------------------------------------------


def test_a_missing_package_names_the_rail_that_builds_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The install gate proves the release-mode package the package rail made.
    Building a debug one here would prove bytes that can never be published."""
    monkeypatch.setattr("capsem.gate.arch.host_system", lambda: "Darwin")
    (tmp_path / "Cargo.toml").write_text(WORKSPACE)

    with pytest.raises(GateError, match="just _cross-compile"):
        InstallGate(RecordingRunner(tmp_path)).run()


def test_a_package_from_another_version_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A stale `dist/` entry would otherwise be installed and proved instead of
    the candidate this checkout describes."""
    root = _macos_checkout(tmp_path, monkeypatch)
    runner = RecordingRunner(
        root,
        replies={"dpkg-deb -f": "1.2.3", "systemctl is-system-running": "running"},
    )

    with pytest.raises(GateError, match=f"declares version 1.2.3, but this checkout is {VERSION}"):
        InstallGate(runner).run()

    assert not runner.ran(r"dpkg -i"), "nothing may be installed after the refusal"


def test_dpkg_reporting_a_different_installed_version_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _macos_checkout(tmp_path, monkeypatch)
    runner = RecordingRunner(
        root,
        replies={
            "dpkg-deb -f": VERSION,
            "dpkg-query": "0.0.1",
            "systemctl is-system-running": "running",
        },
    )

    with pytest.raises(GateError, match="dpkg reports capsem 0.0.1 installed"):
        InstallGate(runner, macos_glowup_report="report.json").run()


def test_a_macos_run_without_its_glowup_report_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Hosted macOS cannot repeat the nested Apple VZ guest boot, so the local
    report is the only evidence that half ran at all."""
    root = _macos_checkout(tmp_path, monkeypatch)

    with pytest.raises(GateError, match="requires the native glow-up report"):
        InstallGate(_recording(root)).run()


def test_a_local_server_that_never_reports_ready_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Continuing here would author the graph against an unserved root, and the
    handoff would name a URL nothing answers."""
    from capsem.gate.installproof import InstallProof

    runner = RecordingRunner(tmp_path, failures=[f"test -f {SERVE_READY_FILE}"])
    proof = InstallProof(runner, "box", LAYOUT, sleep=lambda _seconds: None)

    with pytest.raises(GateError, match="never reported itself ready"):
        proof.stage_local_assets()


def test_the_container_is_torn_down_even_when_the_proof_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The shell used an EXIT trap; this is the same guarantee without `$?`.

    A leaked privileged systemd container holds its image, its volumes, and
    roughly six gigabytes until something else notices.
    """
    root = _macos_checkout(tmp_path, monkeypatch)
    runner = RecordingRunner(
        root,
        replies={"dpkg-deb -f": VERSION, "systemctl is-system-running": "running"},
        failures=["dpkg -i"],
    )

    with pytest.raises(GateError):
        InstallGate(runner, macos_glowup_report="report.json").run()

    # The first `docker rm -f` clears a predecessor before the container
    # starts; the teardown is the last one, and it is the one under test.
    assert runner.last_index_of(r"docker rm -f capsem-install-test") > runner.index_of(
        r"dpkg -i"
    )
    assert runner.ran(r"docker-storage-policy\.py gc --rail install")


def test_a_host_that_boots_a_guest_runs_the_complete_glowup(tmp_path: Path) -> None:
    """`--skip-install` is what a host without a guest falls back to. Sending
    it where the guest works would silently drop half the proof."""
    from capsem.gate.installproof import InstallProof

    runner = RecordingRunner(tmp_path)
    InstallProof(runner, "box", LAYOUT).prove_glowup("/src/x.deb", boots_a_guest=True)

    assert runner.ran(r"local-release-glowup\.py")
    assert not runner.ran(r"--skip-install")


def test_a_host_without_a_guest_skips_only_the_install_half(tmp_path: Path) -> None:
    from capsem.gate.installproof import InstallProof

    runner = RecordingRunner(tmp_path)
    InstallProof(runner, "box", LAYOUT).prove_glowup("/src/x.deb", boots_a_guest=False)

    assert runner.ran(r"local-release-glowup\.py .*--skip-install")
