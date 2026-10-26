"""Official activation requires original-child Capsem VM qualification."""

import shutil
from contextlib import contextmanager, nullcontext
from pathlib import Path
from types import SimpleNamespace

import pytest
import yaml
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.leases import active_path, release_path, retain_generation
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
    assert '--arch "$ARCH" --preflight' in text
    names = [step.get("name") for step in steps]
    assert names.index("Enable required KVM and vsock devices") < names.index("Build the image")


def test_typed_preflight_is_the_single_named_device_authority_after_grant():
    document = yaml.safe_load((ROOT / ".github/workflows/images.yaml").read_text())
    step = next(
        step
        for step in document["jobs"]["image-arch"]["steps"]
        if step.get("name") == "Enable required KVM and vsock devices"
    )
    script = step["run"]
    assert "test -c " not in script, "silent shell device tests must not bypass typed diagnostics"
    assert script.index("install-vm-device-access") < script.index("--preflight")
    assert "VM device access grant failed" in script
    assert "exit 1" in script
    assert not step.get("continue-on-error", False)
    assert "|| true" not in script


def test_real_absent_device_reports_its_exact_name(tmp_path, monkeypatch):
    monkeypatch.setattr(qualify, "host_platform", lambda: "linux/arm64")
    monkeypatch.setattr(qualify, "Path", lambda name: tmp_path / Path(name).name)
    with pytest.raises(ValueError, match=r"missing /dev/kvm$"):
        qualify.preflight("arm64")


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
    copying = False

    @contextmanager
    def finite(*_, **kwargs):
        nonlocal copying
        assert kwargs["force"] is True
        copying = True
        try:
            yield
        finally:
            copying = False

    monkeypatch.setattr(qualify, "leased", finite)
    monkeypatch.setattr(qualify, "preflight", lambda _: None)
    monkeypatch.setattr(qualify, "load_paths", lambda _: paths)
    raw = (source / "blobs/sha256" / digest.split(":")[1]).read_bytes()
    monkeypatch.setattr(qualify.pinned_image, "download_intent", lambda *_, **__: raw)

    def pull(actual, platform, target, *, repository, original_manifest):
        assert copying and original_manifest == raw
        assert active_path(lease)
        assert actual == selected and repository == config.functional.reference_image.repository
        assert platform == ("linux/amd64" if failure == "platform" else "linux/arm64")
        calls.append("pull")
        shutil.copytree(source, target)

    monkeypatch.setattr(qualify.pinned_image, "pull", pull)

    def run(argv, **kwargs):
        assert not copying, "the child gate must never run under the admission machine lease"
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
    monkeypatch.setattr(qualify, "Path", lambda _: SimpleNamespace(exists=lambda: False))
    with pytest.raises(ValueError, match="/dev/kvm"):
        qualify.preflight("arm64")


@pytest.mark.parametrize(
    ("fault", "message"),
    [
        ("regular", "requires character device /dev/kvm$"),
        ("inaccessible", "requires read/write access to /dev/kvm$"),
        ("missing-vsock", "missing /dev/vhost-vsock$"),
    ],
)
def test_both_devices_must_be_character_devices_and_accessible(monkeypatch, fault, message):
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
    with pytest.raises(ValueError, match=message):
        qualify.preflight("arm64")


def test_preflight_only_cannot_accept_candidate_inputs(monkeypatch):
    monkeypatch.setattr(qualify, "preflight", lambda _: pytest.fail("invalid request reached host"))
    with pytest.raises(SystemExit) as error:
        qualify.main(["--arch", "arm64", "--preflight", "--image", "dev"])
    assert error.value.code == 2


def test_nearly_full_protected_stage_refuses_before_any_original_copy(tmp_path, monkeypatch):
    config = load(ROOT)
    policy = load_policy(ROOT)
    settings = config.functional.reference_image
    stages = dict(policy.stages)
    stages[settings.cache_stage] = stages[settings.cache_stage].model_copy(
        update={"max_size_bytes": 5000, "warm_size_bytes": 0}
    )
    paths = CachePaths(
        repository_root=tmp_path, policy=policy.model_copy(update={"stages": stages})
    )
    foreign = paths.stage(settings.cache_stage) / "foreign"
    foreign.mkdir(parents=True)
    (foreign / "original").write_bytes(b"x" * 4990)
    retain_generation(paths, settings.cache_stage, "foreign")
    source, digest, _ = _layout(tmp_path / "source")
    original = tmp_path / "original"
    output(original, digest, repository=settings.repository)
    calls = []
    monkeypatch.setattr(qualify, "load_paths", lambda _: paths)
    monkeypatch.setattr(qualify, "leased", lambda *_, **__: nullcontext())
    monkeypatch.setattr(qualify, "preflight", lambda _: None)
    raw = (source / "blobs/sha256" / digest.split(":")[1]).read_bytes()
    monkeypatch.setattr(
        qualify.pinned_image, "download_intent", lambda *_, **__: raw, raising=False
    )

    def pull(*args, **kwargs):
        calls.append("pull")
        shutil.copytree(source, args[2])

    monkeypatch.setattr(qualify.pinned_image, "pull", pull)
    try:
        with pytest.raises(ValueError):
            qualify.qualify(
                config,
                "dev",
                "arm64",
                settings.repository,
                digest,
                original,
                runner=SimpleNamespace(run=lambda *_, **__: pytest.fail("qualifier reached")),
            )
        assert calls == [], "projected bytes must be refused BEFORE pulling/staging originals"
        assert active_path(paths.stage(settings.cache_stage) / ".foreign.lock")
        assert (foreign / "original").read_bytes() == b"x" * 4990
    finally:
        release_path(paths.stage(settings.cache_stage) / ".foreign.lock")
