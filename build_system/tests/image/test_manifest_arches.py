"""Manifest generation must honor the build's selected architectures."""

import json

import pytest
from capsem_builder.image.docker import generate_checksums


def test_selected_arch_ignores_unselected_partial_build(tmp_path):
    for arch in ("arm64", "x86_64"):
        directory = tmp_path / arch
        directory.mkdir()
        for name in ("vmlinuz", "initrd.img"):
            (directory / name).write_bytes(name.encode())
    (tmp_path / "arm64/rootfs.erofs").write_bytes(b"rootfs")

    path = generate_checksums(tmp_path, "0.13.0", arches=["arm64"])

    manifest = json.loads(path.read_text())
    release = manifest["assets"]["releases"][manifest["assets"]["current"]]
    assert set(release["arches"]) == {"arm64"}
    assert "x86_64" not in (tmp_path / "B3SUMS").read_text()
    assert (tmp_path / "current").readlink().name == "arm64"
    assert sorted(p.name for p in (tmp_path / "x86_64").iterdir()) == [
        "initrd.img", "vmlinuz"
    ]


@pytest.mark.parametrize("missing", ["vmlinuz", "initrd.img", "rootfs.erofs"])
def test_selected_arch_still_requires_every_boot_asset(tmp_path, missing):
    directory = tmp_path / "arm64"
    directory.mkdir()
    for name in ("vmlinuz", "initrd.img", "rootfs.erofs"):
        if name != missing:
            (directory / name).write_bytes(b"asset")
    with pytest.raises(FileNotFoundError, match=missing):
        generate_checksums(tmp_path, "0.13.0", arches=["arm64"])
    assert not (tmp_path / "manifest.json").exists()


def test_missing_selected_arch_cannot_fall_back_to_flat_assets(tmp_path):
    for name in ("vmlinuz", "initrd.img", "rootfs.erofs"):
        (tmp_path / name).write_bytes(b"asset")
    with pytest.raises(FileNotFoundError, match="arm64"):
        generate_checksums(tmp_path, "0.13.0", arches=["arm64"])


@pytest.mark.parametrize("arches", [[], ["../arm64"], ["current"], ["aarch64"]])
def test_invalid_arch_selection_is_rejected_without_writing(tmp_path, arches):
    with pytest.raises(ValueError, match="architecture"):
        generate_checksums(tmp_path, "0.13.0", arches=arches)
    assert not list(tmp_path.iterdir())
