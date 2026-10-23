"""The offline packing process uses its declared tool-image architecture."""

import importlib.util
from pathlib import Path
from unittest.mock import Mock, patch

import pytest
from capsem_builder.image import docker
from capsem_builder.image.models import ContainerNetwork

ROOT = Path(__file__).resolve().parents[3]


def test_erofs_honors_an_explicit_foreign_tool_platform(tmp_path):
    with patch.object(docker, "run_cmd") as run, patch.object(docker, "_native_linux_platform", return_value="linux/arm64"):
        docker.create_erofs("docker", tmp_path / "rootfs.tar", tmp_path / "rootfs.erofs", "lz4hc",
                            tool_image="tools:amd64", tool_platform="linux/amd64",
                            runtime_network=ContainerNetwork.NONE)
    command = run.call_args.args[0]
    assert command[command.index("--platform") + 1] == "linux/amd64"


@pytest.mark.parametrize("architecture", ["arm64", "amd64"])
def test_image_evidence_propagates_the_selected_platform_to_packer(tmp_path, monkeypatch, architecture):
    spec = importlib.util.spec_from_file_location("image_rootfs_platform", ROOT / "images/ci/rootfs.py")
    assert spec and spec.loader
    rootfs = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(rootfs)
    packer = Mock()
    monkeypatch.setattr(rootfs, "asset_tools", lambda *_args: "tools:" + architecture)
    monkeypatch.setattr(rootfs, "from_rootfs", lambda *_args: {})
    monkeypatch.setattr(rootfs, "digest_file", lambda *_args: "0" * 64)
    monkeypatch.setattr(rootfs.docker, "export_container_fs", lambda *_args: None)
    monkeypatch.setattr(rootfs.docker, "generate_cyclonedx_obom", lambda *_args, **_kwargs: None)
    monkeypatch.setattr(rootfs.docker, "create_erofs", packer)
    rootfs.main(["--image", "app:proof", "--arch", architecture, "--repository", "registry.invalid/app",
                 "--digest", "sha256:" + "1" * 64, "--out", str(tmp_path)])
    assert packer.call_args.kwargs["tool_platform"] == "linux/" + architecture
