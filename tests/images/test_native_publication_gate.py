"""Official activation requires original-child Capsem VM qualification."""

import shutil
from pathlib import Path
from types import SimpleNamespace

import pytest
import yaml
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.leases import active_path
from capsem_builder.cache.paths import CachePaths
from capsem_builder.gate.config import load

from images.ci import qualify
from tests.fixtures.oci import producer_evidence, published_root
from tests.fixtures.oci.test_pinned_image import _layout
from tests.fixtures.oci.test_producer_evidence import output

ROOT = Path(__file__).resolve().parents[2]


def test_original_root_is_qualified_before_evidence_or_activation():
    document = yaml.safe_load((ROOT / ".github/workflows/images.yaml").read_text())
    jobs = document["jobs"]
    steps = jobs["image-arch"]["steps"]
    names = [step.get("name") for step in steps]
    pack = names.index("Inventory and pack the rootfs")
    qualify = names.index("Qualify the original platform image under Capsem")
    attach = names.index("Attach the OBOM, package inventory and the EROFS rootfs")
    assert pack < qualify < attach < names.index("Record this architecture")
    step = steps[qualify]
    assert step["if"] == steps[pack]["if"] == steps[attach]["if"]
    assert not step.get("continue-on-error", False)
    assert "images.ci.qualify" in step["run"]
    assert step["env"]["DIGEST"] == "${{ steps.push.outputs.digest }}"
    assert '--output "$RUNNER_TEMP/rootfs"' in step["run"]
    # Original producer bytes must not become untracked source inputs copied
    # into the gate's private source snapshot (or change its source identity).
    assert '--out "$RUNNER_TEMP/rootfs"' in steps[pack]["run"]
    assert steps[attach]["working-directory"] == "${{ runner.temp }}/rootfs"
    assert jobs["image"]["needs"] == "image-arch"
    assert jobs["catalog"]["needs"] == "image"
    assert "needs.image.result == 'success'" in jobs["catalog"]["if"]
    assert not jobs["image"].get("continue-on-error", False)


def test_actual_native_devices_and_existing_setup_precede_image_production():
    document = yaml.safe_load((ROOT / ".github/workflows/images.yaml").read_text())
    steps = document["jobs"]["image-arch"]["steps"]
    text = "\n".join(step.get("run", "") for step in steps)
    assert "install-vm-device-access" in text
    assert "prepare-linux-sandbox.py --repair-hosted-runner" in text
    assert "cargo fetch --locked" in text
    assert "test -c /dev/kvm" in text and "test -c /dev/vhost-vsock" in text
    assert '--arch "$ARCH" --preflight' in text
    names = [step.get("name") for step in steps]
    assert names.index("Enable required KVM and vsock devices") < names.index("Build the image")


@pytest.mark.parametrize("failure", [None, "qualifier", "root", "platform", "subject"])
def test_original_artifact_and_live_lease_through_exact_qualification(
    tmp_path, monkeypatch, failure
):
    config = load(ROOT)
    paths = CachePaths(repository_root=tmp_path, policy=load_policy(ROOT))
    source, digest, _ = _layout(tmp_path / "source")
    repository = config.functional.reference_image.repository
    original = tmp_path / "original"
    output(original, digest, repository=repository)
    selected = "sha256:" + "0" * 64 if failure == "subject" else digest
    key = "official-dev-" + selected.split(":")[1]
    layout = paths.stage(config.functional.reference_image.cache_stage) / key
    lease = layout.parent / ("." + key + ".lock")
    calls = []
    monkeypatch.setattr(qualify, "preflight", lambda _: None)
    monkeypatch.setattr(qualify, "load_paths", lambda _: paths)

    def pull(actual, platform, target, *, repository):
        assert active_path(lease)
        assert actual == selected and repository == config.functional.reference_image.repository
        assert platform == ("linux/amd64" if failure == "platform" else "linux/arm64")
        calls.append("pull")
        shutil.copytree(source, target)

    monkeypatch.setattr(qualify.pinned_image, "pull", pull)

    def run(argv, **kwargs):
        calls.append("qualify")
        assert active_path(lease)
        assert argv[6:] == ["image-qualify", "--image", "dev", "--layout", str(layout), "--slow"]
        assert "--native-report" not in argv
        assert kwargs["cwd"] == config.root
        producer_evidence.ready(
            layout,
            repository,
            "arm64",
            maximum=config.functional.reference_image.evidence_max_bytes,
        )
        assert (
            published_root.ready(layout)[1].read_bytes() == (original / "rootfs.erofs").read_bytes()
        )
        if failure == "qualifier":
            raise RuntimeError("real qualifier failed")
        if failure == "root":
            (original / "rootfs.erofs").write_bytes(b"changed after qualification")

    def invoke():
        qualify.qualify(
            config,
            "dev",
            "amd64" if failure == "platform" else "arm64",
            repository,
            selected,
            original,
            runner=SimpleNamespace(run=run),
        )

    if failure is None:
        invoke()
        assert calls == ["pull", "qualify"]
        assert (original / "rootfs.erofs").read_bytes() == b"ORIGINAL root bytes"
    else:
        with pytest.raises((ValueError, RuntimeError)):
            invoke()
        if failure in ("platform", "subject"):
            assert "qualify" not in calls
    assert not active_path(lease)
    assert original.is_dir(), "qualification failure must not delete original producer evidence"


@pytest.mark.parametrize("platform", ["darwin/arm64", "linux/amd64", "linux/None"])
def test_foreign_or_unknown_host_never_claims_native_arm(monkeypatch, platform):
    monkeypatch.setattr(qualify, "host_platform", lambda: platform)
    with pytest.raises(ValueError, match="actual native"):
        qualify.preflight("arm64")


def test_missing_native_devices_are_a_failure_not_a_skip(monkeypatch):
    monkeypatch.setattr(qualify, "host_platform", lambda: "linux/arm64")
    monkeypatch.setattr(Path, "exists", lambda _: False)
    with pytest.raises(ValueError, match="/dev/kvm"):
        qualify.preflight("arm64")


@pytest.mark.parametrize("fault", ["regular", "inaccessible", "missing-vsock"])
def test_both_devices_must_be_character_devices_and_accessible(monkeypatch, fault):
    monkeypatch.setattr(qualify, "host_platform", lambda: "linux/arm64")
    monkeypatch.setattr(
        qualify,
        "Path",
        lambda name: SimpleNamespace(
            exists=lambda: not (fault == "missing-vsock" and name.endswith("vhost-vsock")),
            stat=lambda: SimpleNamespace(st_mode=0o100600 if fault == "regular" else 0o020600),
        ),
    )
    monkeypatch.setattr(qualify.os, "access", lambda *_: fault != "inaccessible")
    with pytest.raises(ValueError, match="accessible /dev/"):
        qualify.preflight("arm64")


def test_preflight_only_cannot_accept_candidate_inputs(monkeypatch):
    monkeypatch.setattr(qualify, "preflight", lambda _: pytest.fail("invalid request reached host"))
    with pytest.raises(SystemExit) as error:
        qualify.main(["--arch", "arm64", "--preflight", "--image", "dev"])
    assert error.value.code == 2
