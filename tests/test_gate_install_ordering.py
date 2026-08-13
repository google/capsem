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

import json
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner
from helpers.profile_content import materialize_required_artifacts

from capsem.gate import config as gate_config
from capsem.gate.content import LocalInstallContent, ProfileContent, SelectedInstallContent
from capsem.gate.docker import Docker
from capsem.gate.errors import GateError
from capsem.gate.install import InstallGate
from capsem.gate.installproof import InstallProof
from capsem.gate.productschema import ProfileRevisionPolicy
from capsem.gate.releasegraph import ReleaseGraph
from capsem.gate.sourcecommit import SourceCommit

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
INSTALL = CONFIG.install
LAYOUT = INSTALL.layout
SERVE_READY_FILE = INSTALL.serve_ready_file
PREINSTALL_ADMIN = INSTALL.preinstall_admin
SOURCE_COMMIT = SourceCommit("0" * 40)


def test_selected_profile_revision_policy_is_a_typed_install_authority() -> None:
    assert INSTALL.profile_revision_policy is ProfileRevisionPolicy.SELECTED_INPUT


VERSION = "9.9.9"
WORKSPACE = f"""\
[workspace]
members = ["crates/capsem"]

[workspace.package]
version = "{VERSION}"
"""

AUTHORITATIVE = f"{LAYOUT.channel}/{INSTALL.graph_manifest}"


def _checkout(tmp_path: Path, *, dpkg_arch: str) -> Path:
    """A fake checkout carrying the real gate configuration.

    Copied rather than invented: a second configuration here could drift from
    the one the gate runs with, and then these tests would prove an ordering
    nobody executes.
    """
    (tmp_path / "Cargo.toml").write_text(WORKSPACE)
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    (tmp_path / "dist").mkdir()
    (tmp_path / "dist" / f"Capsem_{VERSION}_{dpkg_arch}.deb").write_text("package bytes")
    return tmp_path


def _local_content(root: Path) -> ProfileContent:
    """One internally consistent local cohort, separate from canonical paths."""
    config = gate_config.load(root)
    content = ProfileContent.isolated(
        config,
        root / config.assets.test_root / config.suites.pytest.base_profile,
    )
    manifest = {
        "assets": {
            "current": "test",
            "releases": {
                "test": {"arches": {name: {} for name in config.architectures}},
            },
        },
    }
    payload = __import__("json").dumps(manifest).encode()
    content.assets.mkdir(parents=True)
    (content.assets / config.install.manifest_name).write_bytes(payload)
    materialize_required_artifacts(config, content.assets)
    config_manifest = content.config / config.suites.pytest.test_manifest
    config_manifest.parent.mkdir(parents=True)
    config_manifest.write_bytes(payload)
    profile = content.profiles(config) / config.suites.pytest.base_profile / "profile.toml"
    profile.parent.mkdir(parents=True)
    profile.write_text("name = 'code'\n")
    return content


def _selected_content(root: Path) -> SelectedInstallContent:
    """Add the verified immutable transport required by a release cohort."""
    config = gate_config.load(root)
    content = _local_content(root)
    inputs = content.root / config.install.selected_inputs_dir
    inputs.mkdir()
    payload = inputs / "profile-payload.bin"
    payload.write_bytes(b"selected profile bytes")
    manifest = {
        "assets": {
            "current": "test",
            "releases": {
                "test": {"arches": {name: {} for name in config.architectures}},
            },
        },
        "payload": {"url": payload.as_uri()},
    }
    manifest_bytes = json.dumps(manifest).encode()
    (content.assets / config.install.manifest_name).write_bytes(manifest_bytes)
    (content.config / config.suites.pytest.test_manifest).write_bytes(manifest_bytes)
    (inputs / config.install.manifest_name).write_bytes(manifest_bytes)
    (inputs / config.package.release_inputs_name).write_text("{}\n")
    return SelectedInstallContent(content)


def test_local_content_fixture_tracks_the_config_owned_artifact_inventory(tmp_path: Path) -> None:
    root = _checkout(tmp_path, dpkg_arch=CONFIG.host_arch().dpkg)
    config = gate_config.load(root)
    content = _local_content(root)
    expected = {*config.artifacts.bootable, *config.assets.evidence_artifacts}

    for arch in config.architectures.values():
        assert {path.name for path in (content.assets / arch.name).iterdir()} == expected


@pytest.fixture
def gate(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple[InstallGate, RecordingRunner]:
    """A gate on a fake checkout, whose every command is recorded, not run.

    The container answers "yes" to `test -f` so staging reaches the handoff,
    reports the expected versions, and comes up as a non-Linux host -- the
    macOS shape, which is where the local proof runs.
    """
    root = _macos_checkout(tmp_path, monkeypatch)
    runner = _recording(root)
    return (
        InstallGate(
            runner,
            content=LocalInstallContent(_local_content(root)),
            macos_glowup_report=str(root / "report.json"),
            source_commit=SOURCE_COMMIT,
        ),
        runner,
    )


def _macos_checkout(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A checkout on a macOS host with no colima, the local-proof shape."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: None)
    # Image identity/materialization has its own focused contracts. These
    # tests own only the transaction order after that graph prerequisite has
    # produced an exact image.
    monkeypatch.setattr(
        "capsem.gate.installimage.require_local_image",
        lambda _runner, _config: "sha256:" + "1" * 64,
    )
    return _checkout(tmp_path, dpkg_arch=CONFIG.host_arch().dpkg)


#: What a healthy postinst records: the channel it actually hydrated from.
#: Read back after `dpkg -i`, because `|| apt-get install -f -y` runs the
#: postinst again and a retry must not install from somewhere else.
HYDRATED = f"event=manifest_source source={INSTALL.file_url_scheme}{INSTALL.mount}/{AUTHORITATIVE}"


def _recording(root: Path, *, hydrated: str = HYDRATED) -> RecordingRunner:
    return RecordingRunner(
        root,
        replies={
            "dpkg-deb -f": VERSION,
            "dpkg-query": VERSION,
            "systemctl is-system-running": "running",
            "event=manifest_source": hydrated,
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
    assert runner.ran(rf"--source-commit {SOURCE_COMMIT}")


def test_package_dependency_authority_is_verified_before_install(
    gate: tuple[InstallGate, RecordingRunner],
) -> None:
    built, runner = gate
    built.run()

    runner.assert_order(r"install-deb-runtime-dependencies\.py .*--verify-only", r"dpkg -i")


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
    assert runner.ran(r"--profile-revision-policy selected-input")


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
    graph = ReleaseGraph(Docker(runner), CONFIG, source_commit=SOURCE_COMMIT)

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
    graph = ReleaseGraph(Docker(runner), CONFIG, source_commit=SOURCE_COMMIT)

    with pytest.raises(GateError, match="would find no request"):
        graph.hand_off(AUTHORITATIVE)


def test_clearing_a_handoff_that_was_never_written_does_nothing(tmp_path: Path) -> None:
    """Cleanup runs on the failure path, where the write may not have happened."""
    runner = RecordingRunner(tmp_path)

    ReleaseGraph(Docker(runner), CONFIG, source_commit=SOURCE_COMMIT).clear_handoff()

    assert not runner.ran(r"install-manifest-request")


# ---------------------------------------------------------------------------
# The release-lane shape
# ---------------------------------------------------------------------------


def test_a_release_lane_stages_verified_inputs_and_authors_the_exact_package_graph(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Selected profiles and the exact package become one offline install graph."""
    root = _macos_checkout(tmp_path, monkeypatch)
    runner = _recording(root)
    selected = _selected_content(root)
    content = selected.content

    InstallGate(
        runner,
        content=selected,
        macos_glowup_report=str(root / "report.json"),
        source_commit=SOURCE_COMMIT,
    ).run()

    assert runner.matching(r"cp -R .*assets/\.")
    assert not runner.ran(r"stage-release-test-inputs\.py")
    assert runner.ran(r"serve-release-test-root\.py")
    assert len(runner.matching(r"verify-release-inputs\.py")) == 2, (
        "the fetched graph and input report must be rehashed once on the host "
        "and again through the sealed container's read-only mount"
    )
    assert runner.ran(r"install-manifest-request\.sh write")
    assert runner.ran(r"assets channel build")
    assert runner.ran(r"assets channel record-binary")
    assert runner.ran(r"dpkg-deb --extract")
    assert runner.ran(r"dpkg -i")
    started = runner.matching(r"docker run -d")[0]
    assert f"-v {content.assets}:/src/assets:ro" in started
    assert f"-v {content.config}:/src/target/config:ro" in started
    assert f"-v {content.root}:{content.root}:ro" in started


def test_local_install_mounts_only_the_selected_content_pair(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Docker never sees the mutable checkout selector that it can replace."""
    root = _macos_checkout(tmp_path, monkeypatch)
    content = _local_content(root)
    selected = content.assets
    (root / "assets").symlink_to(selected.relative_to(root))
    canonical = root / "target/config"
    canonical.mkdir(parents=True)
    sentinel = canonical / "stale"
    sentinel.write_text("untouched")
    runner = _recording(root)

    InstallGate(
        runner,
        content=LocalInstallContent(content),
        macos_glowup_report=str(root / "report.json"),
        source_commit=SOURCE_COMMIT,
    ).run()

    started = runner.matching(r"docker run -d")[0]
    assert f"-v {content.assets}:/src/assets:ro" in started
    assert f"-v {content.config}:/src/target/config:ro" in started
    assert f"-v {root / 'assets'}:/src/assets:ro" not in started
    assert f"-v {canonical}:/src/target/config:ro" not in started
    assert (root / "assets").is_symlink()
    assert (root / "assets").readlink() == selected.relative_to(root)
    assert sentinel.read_text() == "untouched"


def test_local_install_without_a_selected_content_pair_fails_before_docker(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _macos_checkout(tmp_path, monkeypatch)
    runner = _recording(root)

    with pytest.raises(GateError, match="selected profile content"):
        InstallGate(
            runner,
            macos_glowup_report=str(root / "report.json"),
            source_commit=SOURCE_COMMIT,
        ).run()

    assert not runner.ran(r"docker build")
    assert not runner.ran(r"docker run -d")


# ---------------------------------------------------------------------------
# Refusals
# ---------------------------------------------------------------------------


def test_a_missing_package_names_the_rail_that_builds_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The install gate proves the release-mode package the package rail made.
    Building a debug one here would prove bytes that can never be published."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    root = _checkout_without_package(tmp_path)

    with pytest.raises(GateError, match="just _cross-compile"):
        InstallGate(RecordingRunner(root), source_commit=SOURCE_COMMIT).run()


def _checkout_without_package(tmp_path: Path) -> Path:
    (tmp_path / "Cargo.toml").write_text(WORKSPACE)
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    return tmp_path


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
        InstallGate(
            runner,
            content=LocalInstallContent(_local_content(root)),
            source_commit=SOURCE_COMMIT,
        ).run()

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

    with pytest.raises(GateError, match=r"dpkg reports capsem 0.0.1 installed"):
        InstallGate(
            runner,
            content=LocalInstallContent(_local_content(root)),
            macos_glowup_report="report.json",
            source_commit=SOURCE_COMMIT,
        ).run()


def test_a_macos_run_without_its_glowup_report_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Hosted macOS cannot repeat the nested Apple VZ guest boot, so the local
    report is the only evidence that half ran at all."""
    root = _macos_checkout(tmp_path, monkeypatch)

    with pytest.raises(GateError, match="requires the native glow-up report"):
        InstallGate(
            _recording(root),
            content=LocalInstallContent(_local_content(root)),
            source_commit=SOURCE_COMMIT,
        ).run()


def test_a_local_server_that_never_reports_ready_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Continuing here would author the graph against an unserved root, and the
    handoff would name a URL nothing answers."""
    runner = RecordingRunner(tmp_path, failures=[f"test -f {SERVE_READY_FILE}"])
    proof = InstallProof(runner, CONFIG, sleep=lambda _seconds: None)

    with pytest.raises(GateError, match="never reported itself ready"):
        proof.stage_content(ProfileContent.standalone(CONFIG))
        proof.start_local_server()


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
        InstallGate(
            runner,
            content=LocalInstallContent(_local_content(root)),
            macos_glowup_report="report.json",
            source_commit=SOURCE_COMMIT,
        ).run()

    # The first `docker rm -f` clears a predecessor before the container
    # starts; the teardown is the last one, and it is the one under test.
    assert runner.last_index_of(r"docker rm -f -v capsem-install-test") > runner.index_of(
        r"dpkg -i"
    )
    assert runner.ran(r"docker-storage-policy\.py gc --rail install")


def test_a_host_that_boots_a_guest_runs_the_complete_glowup(tmp_path: Path) -> None:
    """`--skip-install` is what a host without a guest falls back to. Sending
    it where the guest works would silently drop half the proof."""
    runner = RecordingRunner(tmp_path)
    InstallProof(runner, CONFIG).prove_glowup("/src/x.deb", boots_a_guest=True)

    assert runner.ran(r"local-release-glowup\.py")
    assert runner.ran(r"--profile-revision-policy selected-input")
    assert not runner.ran(r"--skip-install")


def test_glowup_writes_bounded_evidence_through_one_host_mount(
    gate: tuple[InstallGate, RecordingRunner],
) -> None:
    built, runner = gate
    built.run()

    evidence = runner.root / LAYOUT.glowup_evidence
    mounted = f"{evidence}:{INSTALL.mount}/{LAYOUT.glowup_evidence}:rw"
    assert evidence.is_dir()
    assert runner.ran(mounted)
    assert runner.ran(rf"--evidence-dir {LAYOUT.glowup_evidence}")
    assert not runner.ran(rf"{runner.root / LAYOUT.glowup}:[^ ]+:rw")


def test_a_host_without_a_guest_skips_only_the_install_half(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path)
    InstallProof(runner, CONFIG).prove_glowup("/src/x.deb", boots_a_guest=False)

    assert runner.ran(r"local-release-glowup\.py .*--skip-install")


def test_an_install_that_hydrated_from_elsewhere_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The retry hazard, asserted as behaviour rather than as a comment.

    `dpkg -i "<deb>" || apt-get install -f -y` runs the postinst twice. The
    postinst used to drop the handoff on its way out of a failed first attempt,
    so the second one hydrated from the public channel -- and if it had
    succeeded, the gate would have qualified an install of something nobody
    handed it.
    """
    root = _macos_checkout(tmp_path, monkeypatch)
    runner = _recording(
        root,
        hydrated=(
            "event=manifest_source source=https://release.capsem.org/assets/stable/manifest.json"
        ),
    )

    with pytest.raises(GateError, match="a channel the gate did not hand it"):
        InstallGate(
            runner,
            content=LocalInstallContent(_local_content(root)),
            macos_glowup_report=str(root / "report.json"),
            source_commit=SOURCE_COMMIT,
        ).run()


def test_an_install_that_recorded_no_source_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Silence is not proof. A postinst that recorded nothing leaves the gate
    unable to say which channel it qualified, which is the same hazard with the
    evidence missing instead of wrong."""
    root = _macos_checkout(tmp_path, monkeypatch)
    runner = _recording(root, hydrated="")

    with pytest.raises(GateError, match="recorded no manifest source"):
        InstallGate(
            runner,
            content=LocalInstallContent(_local_content(root)),
            macos_glowup_report=str(root / "report.json"),
            source_commit=SOURCE_COMMIT,
        ).run()


# ---------------------------------------------------------------------------
# The native macOS proof, handed over
# ---------------------------------------------------------------------------


def test_the_macos_report_is_found_where_the_tart_step_writes_it(tmp_path) -> None:
    """A macOS host cannot boot a guest inside the Linux install container, so
    the native Tart proof stands in for it -- and the install rail refuses
    without that report.

    It read `CAPSEM_MACOS_NATIVE_GLOWUP_REPORT` and nothing set it. The path
    was declared in `[modules]` the whole time and the Tart step wrote exactly
    there, so a complete local gate always failed at the last step with
    "requires the native glow-up report from this module".
    """
    from capsem.gate import config as gate_config
    from capsem.gate.install import macos_report

    config = gate_config.load(PROJECT_ROOT)
    written = config.path(config.modules.macos_glowup_report)

    assert macos_report(config, environ={}) in (None, str(written))

    # With the report on disk and no variable, the rail finds it.
    fake = tmp_path / config.modules.macos_glowup_report
    fake.parent.mkdir(parents=True)
    fake.write_text("{}", encoding="utf-8")
    local = gate_config.load(PROJECT_ROOT).model_copy(update={"root": tmp_path})
    assert macos_report(local, environ={}) == str(fake)


def test_a_release_lane_may_hand_the_report_over_by_variable(tmp_path) -> None:
    """CI produces it in another job, so the variable still wins."""
    from capsem.gate import config as gate_config
    from capsem.gate.install import macos_report

    config = gate_config.load(PROJECT_ROOT)
    handed = str(tmp_path / "elsewhere.json")

    assert macos_report(config, environ={config.modules.macos_report_variable: handed}) == handed


def test_neither_present_still_refuses(tmp_path) -> None:
    """The refusal is right when the proof genuinely did not run."""
    from capsem.gate import config as gate_config
    from capsem.gate.install import macos_report

    config = gate_config.load(PROJECT_ROOT).model_copy(update={"root": tmp_path})

    assert macos_report(config, environ={}) is None
