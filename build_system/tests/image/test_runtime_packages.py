"""The runtime owns the container launcher's tools; no profile may.

runc, umoci, python3, iptables and iproute2 came from the profiles' apt lists,
so the runtime could run an image only while some profile happened to install
them. Deleting profiles (#289) would have removed the container path with
them. The runtime now declares them in `[build.rootfs] runtime_apt_packages`
and the builder refuses a profile that lists one.
"""

import tomllib
from pathlib import Path

import pytest
from capsem_builder.image.docker import _rootfs_context

from .test_docker import _profile_guest_config

PROJECT_ROOT = Path(__file__).resolve().parents[3]
LAUNCHER_TOOLS = {"runc", "umoci", "python3", "iptables", "iproute2"}


def test_the_runtime_declares_the_launchers_tools():
    build = tomllib.loads((PROJECT_ROOT / "config/docker/image/build.toml").read_text())
    assert set(build["build"]["rootfs"]["runtime_apt_packages"]) >= LAUNCHER_TOOLS


@pytest.mark.parametrize("profile", sorted(p.name for p in (PROJECT_ROOT / "config/profiles").iterdir()))
def test_no_profile_lists_a_runtime_package(profile):
    listed = PROJECT_ROOT / "config/profiles" / profile / "apt-packages.txt"
    if not listed.is_file():
        return
    packages = {line.strip() for line in listed.read_text().splitlines() if line.strip()}
    assert not packages & LAUNCHER_TOOLS, f"{profile} lists runtime packages"


def test_the_rootfs_installs_the_runtime_packages_first(tmp_path):
    config = _profile_guest_config(tmp_path, "code")
    packages = _rootfs_context(config, "arm64")["apt_packages"]
    runtime = list(config.build.rootfs.runtime_apt_packages)
    assert packages[: len(runtime)] == runtime
    assert len(packages) == len(set(packages))


def test_a_profile_claiming_a_runtime_package_is_refused(tmp_path):
    config = _profile_guest_config(tmp_path, "code")
    apt = config.package_sets["apt"]
    claimed = config.model_copy(
        update={
            "package_sets": {
                **config.package_sets,
                "apt": apt.model_copy(update={"packages": [*apt.packages, "runc"]}),
            }
        }
    )
    with pytest.raises(ValueError, match="belong to the runtime"):
        _rootfs_context(claimed, "arm64")
