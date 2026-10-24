"""Fixed native builds reuse canonical producers; all machine work is doubled."""

import shutil
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.cache.leases import active_path
from capsem_builder.gate.config import load

from tests.fixtures.oci import native_build, native_inputs
from tests.fixtures.oci.test_native_inputs import oci

ROOT = Path(__file__).resolve().parents[3]


def test_candidates_are_fixed_build_specs_not_functional_pin_substitutes():
    config = load(ROOT)
    for name in ("dev", *config.functional.native_images.apps):
        image = native_build.candidate(config, name)
        assert image.settings().name == name
        assert image.section == ("reference_image" if name == "dev" else name)
        with pytest.raises(ValueError, match="pin"):
            image.pinned()
    for name in ("claude-desktop", "../foreign", "unknown"):
        with pytest.raises(ValueError):
            native_build.candidate(config, name)


def base_fixture(monkeypatch, tmp_path):
    config = load(ROOT).model_copy(update={"root": tmp_path})
    stage = tmp_path / "cache"
    stage.mkdir()
    paths = SimpleNamespace(
        stage=lambda _: stage,
        policy=SimpleNamespace(
            stages={"test-images": SimpleNamespace(lease_template=".{key}.lock")}
        ),
    )
    monkeypatch.setattr(native_build, "load_paths", lambda _: paths)
    monkeypatch.setattr(native_build, "dockercurrent", SimpleNamespace(record=lambda *a, **k: None))

    def key(root, name, base=None):
        return ("a" if name == "base" else "b") * 64

    monkeypatch.setattr(native_build, "workflow_key", key)
    monkeypatch.setattr(native_inputs, "workflow_key", key)
    original = tmp_path / "original"
    subject, manifest, selected = oci(original)
    calls = []

    def run(args, **kwargs):
        calls.append(args)
        if "--output" in args:
            destination = Path(args[args.index("--output") + 1].split("dest=")[1])
            shutil.copytree(original, destination)
        return SimpleNamespace(returncode=0)

    monkeypatch.setattr(native_build, "subprocess", SimpleNamespace(run=run))
    monkeypatch.setattr(
        native_build,
        "_retained_image",
        lambda *a, **k: (subject, manifest["config"]["digest"], selected["rootfs"]["diff_ids"]),
    )
    return config, stage, subject, manifest, selected, calls


def test_shared_base_is_built_once_verified_and_leased_through_consumer(monkeypatch, tmp_path):
    config, stage, subject, _, _, calls = base_fixture(monkeypatch, tmp_path)
    with native_build.shared_base(config, "linux/amd64") as (immutable, binding):
        assert immutable == subject and binding.subject == subject
        assert active_path(stage / (".native-base-amd64-" + "a" * 64 + ".lock"))
    assert not active_path(stage / (".native-base-amd64-" + "a" * 64 + ".lock"))
    with native_build.shared_base(config, "linux/amd64") as (_, reused):
        assert reused == binding
    assert sum("--output" in args for args in calls) == 1
    assert all(args[:3] == ["docker", "buildx", "build"] for args in calls)


@pytest.mark.parametrize("fault", ["config", "layers", "platform"])
def test_shared_base_refuses_actual_docker_or_platform_mismatch(monkeypatch, tmp_path, fault):
    config, _, subject, manifest, selected, _ = base_fixture(monkeypatch, tmp_path)
    monkeypatch.setattr(
        native_build,
        "_retained_image",
        lambda *a, **k: (
            subject,
            "sha256:" + "f" * 64 if fault == "config" else manifest["config"]["digest"],
            ["sha256:" + "f" * 64] if fault == "layers" else selected["rootfs"]["diff_ids"],
        ),
    )
    with (
        pytest.raises(ValueError),
        native_build.shared_base(config, "linux/arm64" if fault == "platform" else "linux/amd64"),
    ):
        pytest.fail("foreign base was admitted")


def test_non_native_producer_refuses_before_any_base_or_product_work(monkeypatch):
    config = load(ROOT)
    monkeypatch.setattr(native_build, "host_platform", lambda: "darwin/arm64")

    def unexpected(*args, **kwargs):
        pytest.fail("non-native candidate entered producer")

    monkeypatch.setattr(native_build, "shared_base", unexpected)
    with (
        pytest.raises(ValueError, match="native"),
        native_build.produce(config, "codex-cli", "linux/amd64"),
    ):
        pytest.fail("non-native producer admitted")


@pytest.mark.parametrize("fault", [None, "loaded-config", "skip-validation"])
def test_canonical_product_callback_and_source_lease_span_receipt_consumer(
    monkeypatch, tmp_path, fault
):
    config, stage, base, manifest, selected, _ = base_fixture(monkeypatch, tmp_path)
    child = tmp_path / "child"
    digest, child_manifest, child_config = oci(
        child, prefix=manifest["layers"], diff_prefix=selected["rootfs"]["diff_ids"]
    )
    monkeypatch.setattr(native_build, "host_platform", lambda: "linux/amd64")

    def inspect(runtime, tag, *args, **kwargs):
        chosen, data = (
            (child_manifest, child_config) if "codex-cli" in tag else (manifest, selected)
        )
        return (
            digest if "codex-cli" in tag else base,
            "sha256:" + "f" * 64
            if fault == "loaded-config" and "codex-cli" in tag
            else chosen["config"]["digest"],
            data["rootfs"]["diff_ids"],
        )

    monkeypatch.setattr(native_build, "_retained_image", inspect)

    def producer(image, platform, *, base_image, validate):
        assert base_image == base and platform == "linux/amd64"
        if fault != "skip-validation":
            validate(child, digest, platform, "capsem-native-codex-cli:amd64")
        shutil.copytree(child, image.layout_path(digest))
        return digest

    monkeypatch.setattr(native_build.image_build, "build", producer)
    lease = stage / (".codex-cli-" + digest[7:] + ".lock")
    if fault:
        with pytest.raises(ValueError), native_build.produce(config, "codex-cli", "linux/amd64"):
            pytest.fail("invalid producer admitted")
    else:
        with native_build.produce(config, "codex-cli", "linux/amd64") as (image, subject, binding):
            assert subject == digest and image.settings().name == "codex-cli"
            assert binding.base.subject == base and active_path(lease)
    assert not active_path(lease)


@pytest.mark.parametrize("fault", ["link", "partial", "consumer"])
def test_base_failure_releases_only_its_lease_and_refuses_unsafe_reuse(
    monkeypatch, tmp_path, fault
):
    config, stage, _, _, _, _ = base_fixture(monkeypatch, tmp_path)
    generation = stage / ("native-base-amd64-" + "a" * 64)
    lease = stage / ("." + generation.name + ".lock")
    generation.mkdir()
    if fault == "link":
        (generation / "layout").symlink_to(tmp_path / "original", target_is_directory=True)
    if fault == "partial":
        (generation / "partial").mkdir()
    with pytest.raises(ValueError), native_build.shared_base(config, "linux/amd64"):
        raise ValueError("consumer refused")
    assert not active_path(lease)
    assert (tmp_path / "original/index.json").is_file()
