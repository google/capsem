"""Producer-local aliases belong to the existing Docker cache inventory."""

from pathlib import Path

from capsem_builder.cache.config import load_policy
from capsem_builder.cache.runtimemodels import DockerRuntimePolicy

from tests.fixtures.oci import image_build
from tests.fixtures.oci.test_producer_evidence import setup

ROOT = Path(__file__).resolve().parents[3]


def owned_prefixes():
    policy = load_policy(ROOT).runtimes["docker"]
    assert isinstance(policy, DockerRuntimePolicy)
    return policy.image_prefixes


def test_development_build_and_reuse_tags_are_owned_without_claiming_foreign_dev(
    monkeypatch, tmp_path, capsys
):
    from images.ci import rootfs

    image, digest, calls, _ = setup(monkeypatch, tmp_path)
    settings = image.settings()
    settings.name = "dev"
    settings.local_name = "capsem-dev"
    settings.base_context = "images/base"
    prefixes = owned_prefixes()
    assert not any("dev:arm64".startswith(prefix) for prefix in prefixes)
    original = rootfs.main
    consumed = []

    def producer(args):
        consumed.append(args[args.index("--image") + 1])
        return original(args)

    monkeypatch.setattr(rootfs, "main", producer)
    assert image_build.build(image) == digest
    tags = [call[call.index("--tag") + 1] for call in calls if "--tag" in call]
    assert tags == ["capsem-dev-base:arm64", "capsem-dev:arm64"]
    assert all(any(tag.startswith(prefix) for prefix in prefixes) for tag in tags)
    assert consumed == ["capsem-dev:arm64"]
    assert "docker: capsem-dev:arm64" in capsys.readouterr().out
    calls.clear()
    assert image_build.build(image) == digest
    tags = [call[call.index("--tag") + 1] for call in calls if "--tag" in call]
    assert tags == ["capsem-dev-base:arm64", "capsem-dev:arm64"]
    assert consumed == ["capsem-dev:arm64"], "reuse must not repack original evidence"


def test_both_configured_fixture_local_names_are_owned_and_product_names_are_unchanged():
    from capsem_builder.gate.config import load

    config = load(ROOT)
    prefixes = owned_prefixes()
    for section, name in [("debug_image", "capsem-debug"), ("reference_image", "dev")]:
        settings = getattr(config.functional, section)
        assert settings.name == name
        assert any(settings.local_name.startswith(prefix) for prefix in prefixes)
