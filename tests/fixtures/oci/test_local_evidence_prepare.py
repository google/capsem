"""Local evidence uses the canonical producer and exact retained image identity."""
from __future__ import annotations

import json
from types import SimpleNamespace

import pytest

from tests.fixtures.oci import image_build
from tests.fixtures.oci.test_pinned_image import _layout


def fixture(tmp_path):
    layout, digest, _ = _layout(tmp_path)
    entry = json.loads((layout / "index.json").read_text())["manifests"][0]
    manifest = json.loads((layout / "blobs/sha256" / entry["digest"].split(":")[1]).read_text())
    config = manifest["config"]["digest"]
    image = SimpleNamespace(ready=lambda: layout, pinned=lambda: digest, platform_name=lambda: "linux/arm64", settings=lambda: SimpleNamespace(name="dev"))
    return image, digest, config


def test_missing_or_mismatched_retained_image_fails_before_export(monkeypatch, tmp_path):
    image, digest, _config = fixture(tmp_path)
    calls = []
    def inspect(*args, **kwargs):
        calls.append(args[0])
        return SimpleNamespace(returncode=1, stdout="")
    monkeypatch.setattr(image_build.subprocess, "run", inspect)
    with pytest.raises(ValueError, match="retained"):
        image_build.prepare_local_evidence(image, f"127.0.0.1:12345/library/dev@{digest}", tmp_path / "evidence", inspect_timeout=5)
    assert len(calls) == 1
    monkeypatch.setattr(image_build.subprocess, "run", lambda *_, **__: SimpleNamespace(returncode=0, stdout=json.dumps("sha256:" + "0" * 64)))
    with pytest.raises(ValueError, match="identity"):
        image_build.prepare_local_evidence(image, f"127.0.0.1:12345/library/dev@{digest}", tmp_path / "evidence", inspect_timeout=5)


def test_callback_generates_exact_local_subject_with_existing_producer(monkeypatch, tmp_path):
    from images.ci import rootfs
    image, digest, config = fixture(tmp_path)
    calls = []
    monkeypatch.setattr(image_build.subprocess, "run", lambda *_, **__: SimpleNamespace(returncode=0, stdout=json.dumps(config)))
    monkeypatch.setattr(rootfs, "build_config", lambda: None)
    monkeypatch.setattr(rootfs, "builder_architecture", lambda *_: "arm64")
    monkeypatch.setattr(rootfs.assettools, "image_tag", lambda *_: "existing-pinned-tools")
    def produce(args, *, prepared_tools):
        assert prepared_tools == config
        calls.append(args)
        output = tmp_path / "evidence"
        output.mkdir()
        reference = f"127.0.0.1:12345/library/dev@{digest}"
        (output / "packages.json").write_text(json.dumps({"subject": reference, "architecture": "arm64", "packages": [{"name": "git", "version": "test-real-producer-spy"}]}))
        (output / "obom.cdx.json").write_text(json.dumps({"metadata": {"component": {"name": reference.split("@")[0], "version": digest}}}))
    monkeypatch.setattr(rootfs, "main", produce)
    reference = f"127.0.0.1:12345/library/dev@{digest}"
    result = image_build.prepare_local_evidence(image, reference, tmp_path / "evidence", inspect_timeout=5)
    assert result == tmp_path / "evidence"
    assert len(calls) == 1
    args = calls[0]
    assert args[args.index("--image") + 1] == config
    assert args[args.index("--repository") + 1] == reference.split("@")[0]
    assert args[args.index("--digest") + 1] == digest
    assert args[args.index("--out") + 1] == str(result)


def test_missing_tools_or_foreign_subject_never_relabels_evidence(monkeypatch, tmp_path):
    from images.ci import rootfs
    image, digest, config = fixture(tmp_path)
    reference = f"127.0.0.1:12345/library/dev@{digest}"
    calls = []
    def inspect(*_, **__):
        calls.append(True)
        return SimpleNamespace(returncode=0 if len(calls) == 1 else 1, stdout=json.dumps(config))
    monkeypatch.setattr(image_build.subprocess, "run", inspect)
    monkeypatch.setattr(rootfs, "build_config", lambda: None)
    monkeypatch.setattr(rootfs, "builder_architecture", lambda *_: "arm64")
    monkeypatch.setattr(rootfs.assettools, "image_tag", lambda *_: "missing-tools")
    with pytest.raises(ValueError, match="retained"):
        image_build.prepare_local_evidence(image, reference, tmp_path / "evidence", inspect_timeout=5)
    assert len(calls) == 2
    for wrong in (reference.replace("127.0.0.1", "ghcr.io"), reference.replace(digest, "sha256:" + "0" * 64)):
        with pytest.raises(ValueError, match="identity"):
            image_build.prepare_local_evidence(image, wrong, tmp_path / "evidence", inspect_timeout=5)
    with pytest.raises(ValueError, match="finite"):
        image_build.prepare_local_evidence(image, reference, tmp_path / "evidence", inspect_timeout=float("inf"))


def test_prepared_producer_uses_immutable_tools_without_building(monkeypatch, tmp_path):
    from images.ci import rootfs
    identity = "sha256:" + "1" * 64
    tools = "sha256:" + "2" * 64
    config = SimpleNamespace(architectures={"arm64": SimpleNamespace(docker_platform="linux/arm64")}, asset_tools=SimpleNamespace(runtime_network="none"), erofs={})
    monkeypatch.setattr(rootfs, "build_config", lambda: config)
    monkeypatch.setattr(rootfs, "builder_architecture", lambda *_: "arm64")
    def forbidden(*_):
        raise AssertionError("prepared tools must never build or fetch")
    monkeypatch.setattr(rootfs, "asset_tools", forbidden)
    calls = []
    monkeypatch.setattr(rootfs.docker, "export_container_fs", lambda _, selected, __, target: (calls.append(selected), target.write_bytes(b"export")))
    monkeypatch.setattr(rootfs, "from_rootfs", lambda _, subject, architecture: {"subject": subject, "architecture": architecture})
    monkeypatch.setattr(rootfs.docker, "generate_cyclonedx_obom", lambda _, target, **kwargs: (calls.append(kwargs["tool_image"]), target.write_text('{}')))
    monkeypatch.setattr(rootfs.docker, "experimental_erofs_build_config", lambda *_: (0, "none", 0, 0))
    monkeypatch.setattr(rootfs.docker, "create_erofs", lambda _, __, target, *args, **kwargs: (calls.append(kwargs["tool_image"]), target.write_bytes(b"filesystem")))
    output = tmp_path / "producer"
    rootfs.main(["--image", identity, "--arch", "arm64", "--repository", "127.0.0.1:12345/library/dev", "--digest", "sha256:" + "3" * 64, "--out", str(output)], prepared_tools=tools)
    assert calls == [identity, tools, tools]
    assert (output / "packages.json").is_file() and (output / "obom.cdx.json").is_file()
    assert not (output / "rootfs.tar").exists()


def test_callback_rejects_foreign_producer_output(monkeypatch, tmp_path):
    from images.ci import rootfs
    image, digest, config = fixture(tmp_path)
    monkeypatch.setattr(image_build.subprocess, "run", lambda *_, **__: SimpleNamespace(returncode=0, stdout=json.dumps(config)))
    monkeypatch.setattr(rootfs, "build_config", lambda: None)
    monkeypatch.setattr(rootfs, "builder_architecture", lambda *_: "arm64")
    monkeypatch.setattr(rootfs.assettools, "image_tag", lambda *_: "existing-tools")
    reference = f"127.0.0.1:12345/library/dev@{digest}"
    def foreign(args, **kwargs):
        output = tmp_path / "evidence"
        output.mkdir()
        (output / "packages.json").write_text(json.dumps({"subject": reference, "architecture": "arm64"}))
        (output / "obom.cdx.json").write_text(json.dumps({"metadata": {"component": {"name": "ghcr.io/google/capsem/dev", "version": digest}}}))
    monkeypatch.setattr(rootfs, "main", foreign)
    with pytest.raises(ValueError, match="OBOM subject"):
        image_build.prepare_local_evidence(image, reference, tmp_path / "evidence", inspect_timeout=5)
