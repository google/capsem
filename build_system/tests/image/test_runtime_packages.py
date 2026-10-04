"""The runtime rootfs installs exactly its declared Debian packages.

Applications come from OCI images (#289), so `[build.rootfs]
runtime_apt_packages` is the whole package set of the VM runtime: the
container launcher's tools, what capsem-init needs, the system trust store
and what the in-guest diagnostics run with. Nothing else may reach the
rendered dependency Dockerfile.
"""

import re
import tomllib
from pathlib import Path

import pytest
from capsem_builder.image.config import load_guest_config
from capsem_builder.image.docker import render_dockerfile

PROJECT_ROOT = Path(__file__).resolve().parents[3]
IMAGE_CONFIG = PROJECT_ROOT / "config" / "docker" / "image"
LAUNCHER_TOOLS = {"runc", "umoci", "python3", "iptables", "iproute2"}
INIT_TOOLS = {"auditd", "e2fsprogs"}


def _declared() -> list[str]:
    build = tomllib.loads((IMAGE_CONFIG / "build.toml").read_text())
    return build["build"]["rootfs"]["runtime_apt_packages"]


def test_the_runtime_declares_the_launcher_and_init_tools():
    assert set(_declared()) >= LAUNCHER_TOOLS | INIT_TOOLS | {"ca-certificates"}


@pytest.mark.parametrize("arch", ["arm64", "x86_64"])
def test_the_dependency_dockerfile_installs_exactly_the_runtime_packages(arch):
    config = load_guest_config(IMAGE_CONFIG)
    rendered = render_dockerfile(config.build.asset_dependencies.rootfs_template, config, arch)
    install = re.search(
        r"apt-get install -y --no-install-recommends \\\n(?P<body>.*?) && \\\n",
        rendered,
        re.DOTALL,
    )
    assert install is not None, rendered
    packages = [
        token
        for line in install.group("body").splitlines()
        for token in line.replace("\\", " ").split()
    ]
    assert packages == _declared()
    assert rendered.count("apt-get install") == 1
    for retired in ("npm", "uv pip", "node", "python-requirements", "vim"):
        assert retired not in rendered, retired
