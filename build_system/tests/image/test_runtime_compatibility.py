"""Runtime manifests describe the host releases that can boot this guest."""

import json

import pytest

from capsem_builder.image.docker import generate_checksums


@pytest.fixture
def assets(tmp_path):
    arch = tmp_path / "x86_64"
    arch.mkdir()
    for name in ("vmlinuz", "initrd.img", "rootfs.erofs"):
        (arch / name).write_bytes(name.encode())
    return tmp_path


@pytest.mark.parametrize(
    ("binary_version", "minimum"), [("0.7.3", "0.7.3"), ("0.7.4", "0.7.3"), ("0.8.2", "0.8.0")]
)
def test_runtime_manifest_keeps_the_compatible_host_floor(assets, binary_version, minimum):
    generated = generate_checksums(assets, binary_version)
    runtime = json.loads(generated.read_text())["assets"]
    assert runtime["releases"][runtime["current"]]["min_binary"] == minimum


def test_runtime_manifest_refuses_a_build_version_below_the_guest_floor(assets):
    with pytest.raises(ValueError, match=r"0\.7\.2.*0\.7\.3"):
        generate_checksums(assets, "0.7.2")
    assert not (assets / "manifest.json").exists()
    assert not (assets / "B3SUMS").exists()
