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


DEBUG_IMAGE = PROJECT_ROOT / "images" / "capsem-debug" / "Dockerfile"

#: Packages the runtime and capsem-debug both install, each because the
#: runtime itself needs it -- never because a test does.
SHARED_WITH_DEBUG_IMAGE = {
    "ca-certificates": "the trust store the Capsem CA is installed into",
    "curl": "the in-guest network diagnostics",
    "iproute2": "the container launcher's veth and routes",
    "procps": "capsem-doctor's process checks",
    "python3": "the container launcher and capsem-doctor",
    "python3-pytest": "capsem-doctor, the `capsem doctor` diagnostic users run",
    "python3-rich": "capsem-bench's report",
    "python3-venv": "capsem-doctor's hermetic pip probe",
}

RUNTIME_TOOLING_RATIONALE = """\
Test tooling belongs in capsem-debug, not in the VM runtime.

The runtime rootfs is one minimal image (#289); a test that needs a runner, a
network tool, a package manager, a model SDK or an agent CLI opens a
capsem-debug session (tests/helpers/debug_session.py) and runs it there. A
package both install must be one the runtime needs for itself, listed with
that reason in SHARED_WITH_DEBUG_IMAGE. See tests/fixtures/oci/README.md.
"""


def _debug_image_packages() -> set[str]:
    text = DEBUG_IMAGE.read_text().replace("\\\n", " ")
    match = re.search(r"apt-get install -y --no-install-recommends ([^;]+);", text)
    assert match is not None, "images/capsem-debug/Dockerfile installs no Debian packages"
    return set(match.group(1).split())


def test_the_runtime_carries_no_test_tooling_the_debug_image_does():
    debug = _debug_image_packages()
    assert {"iperf3", "wrk", "dnsutils", "python3-pip"} <= debug
    shared = set(_declared()) & debug
    assert shared <= set(SHARED_WITH_DEBUG_IMAGE), (
        f"{sorted(shared - set(SHARED_WITH_DEBUG_IMAGE))}\n{RUNTIME_TOOLING_RATIONALE}"
    )
