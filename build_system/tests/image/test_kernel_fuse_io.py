"""The guest kernel must honor negotiated FUSE I/O without excess read-ahead."""

import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def test_pinned_kernel_backports_negotiated_fuse_io_size():
    config = tomllib.loads((ROOT / "config/docker/image/build.toml").read_text())
    name = "kernel/patches/0003-fuse-honor-negotiated-io-pages.patch"
    assert name in config["build"]["kernel"]["patches"], (
        "The 6.18 FUSE backing device otherwise splits explicit 1-MiB reads "
        "into 128-KiB round trips regardless of max_pages negotiation."
    )
    patch = (ROOT / "config/docker/image" / name).read_text()
    assert "+\t\tfm->sb->s_bdi->io_pages = fc->max_pages;" in patch
    assert not any("ra_pages =" in line for line in patch.splitlines() if line.startswith("+"))
    assert "98b4ca2378e1f6b6c06a74f699623ebecfb3549d" in patch
