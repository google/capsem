"""Prove the public installer under the shared real systemd harness."""

from __future__ import annotations

import shlex
import shutil
import signal
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from ...gate import installbuilder
from ...gate.config import GateConfig, load
from ...gate.docker import Docker
from ...gate.errors import GateError
from ...gate.installcontainer import start_systemd_container
from ...gate.proc import Runner
from ...policy.dockerpolicy import ContainerNetwork
from . import repository_root


@contextmanager
def public_install_container(
    root: Path, *, image: str | None = None,
) -> Iterator[tuple[Docker, str, GateConfig]]:
    """Own one fresh public-network container without widening sealed proofs."""
    if shutil.which("docker") is None:
        raise OSError("docker is required for --docker-linux-install")
    config = load(root)
    runner = Runner(root)
    docker = Docker(runner)
    settings = config.install
    image = image or installbuilder.require_current(runner, config).image_reference
    user = settings.guest_user
    ready_image = docker.probe(
        image=image, network=ContainerNetwork.NONE,
        command=["bash", "-c", (
            f"test -x {shlex.quote(settings.systemd_command)} && "
            f"id {shlex.quote(user.name)} >/dev/null && "
            "test /dev/null = \"$(readlink /etc/systemd/system/systemd-binfmt.service)\" && "
            f"test ! -e {shlex.quote(settings.installed_capsem)} && "
            f"test ! -e {shlex.quote(user.home + '/' + settings.capsem_home + '/bin/capsem')}"
        )],
    )
    if not ready_image:
        raise GateError("public install requires a clean prepared systemd image and its native test user")
    name = f"{settings.container}-public-{uuid.uuid4().hex[:12]}"
    previous = signal.getsignal(signal.SIGTERM)

    def interrupted(_signal: int, _frame: object) -> None:
        raise SystemExit("public install verification interrupted")

    signal.signal(signal.SIGTERM, interrupted)
    try:
        start_systemd_container(
            docker, settings, name=name, image=image,
            command=[settings.systemd_command], network=ContainerNetwork.BRIDGE,
        )
        yield docker, name, config
    finally:
        try:
            docker.remove(name)
        finally:
            signal.signal(signal.SIGTERM, previous)


def install_script(
    config: GateConfig, *, release_base_url: str, install_script_url: str,
    stable_manifest_url: str, nightly_manifest_url: str, expected_version: str,
    channel_switch: bool, upgrade: bool,
) -> str:
    """Keep package, service and optional transition assertions in one script."""
    quote = shlex.quote
    proof = config.package.proof
    binaries = " ".join(map(quote, proof.binaries))
    versioned = " ".join(map(quote, proof.versioned_binaries))
    script = f"""
set -euo pipefail
export CAPSEM_HOME="$HOME/{config.install.capsem_home}"
export CAPSEM_RUN_DIR="$CAPSEM_HOME/run"
curl -fsSL {quote(install_script_url)} | CAPSEM_CHANNEL=stable CAPSEM_RELEASE_BASE_URL={quote(release_base_url)} sh
test -x "$CAPSEM_HOME/bin/capsem"
"$CAPSEM_HOME/bin/capsem" --version | grep -F {quote(expected_version)}
test -f "$CAPSEM_HOME/assets/{config.install.manifest_name}"
grep -F {quote(stable_manifest_url)} "$CAPSEM_HOME/assets/manifest-metadata.json"
dpkg-query -W -f='${{Version}}' capsem | grep -Fx {quote(expected_version)}
for bin in {binaries}; do
  test -x "$CAPSEM_HOME/bin/$bin"
done
for bin in {versioned}; do
  "$CAPSEM_HOME/bin/$bin" --version
done
"$CAPSEM_HOME/bin/capsem" status | tee "$CAPSEM_HOME/service-status.txt"
"""
    script += "\n".join(
        f'grep -F {quote(expected)} "$CAPSEM_HOME/service-status.txt"'
        for expected in proof.status_requires
    ) + "\n"
    channels = quote(f"{release_base_url.rstrip('/')}/channels.json")
    if channel_switch:
        script += f"""
CAPSEM_RELEASE_CHANNELS_URL={channels} "$CAPSEM_HOME/bin/capsem" update --assets --channel nightly
grep -F {quote(nightly_manifest_url)} "$CAPSEM_HOME/assets/manifest-metadata.json"
CAPSEM_RELEASE_CHANNELS_URL={channels} "$CAPSEM_HOME/bin/capsem" update --assets --channel stable
grep -F {quote(stable_manifest_url)} "$CAPSEM_HOME/assets/manifest-metadata.json"
"""
    if upgrade:
        script += f"""
CAPSEM_RELEASE_CHANNELS_URL={channels} DEBIAN_FRONTEND=noninteractive "$CAPSEM_HOME/bin/capsem" update --yes --channel nightly
"""
    return script


def run_docker_install_smoke(
    *, release_base_url: str, install_script_url: str, stable_manifest_url: str,
    nightly_manifest_url: str, expected_version: str, channel_switch: bool,
    upgrade: bool, docker_image: str | None,
) -> None:
    with public_install_container(repository_root(), image=docker_image) as (docker, name, config):
        script = install_script(
            config, release_base_url=release_base_url, install_script_url=install_script_url,
            stable_manifest_url=stable_manifest_url, nightly_manifest_url=nightly_manifest_url,
            expected_version=expected_version, channel_switch=channel_switch, upgrade=upgrade,
        )
        user = config.install.guest_user
        docker.shell(name, script, user=user.name, env={
            "HOME": user.home, "XDG_RUNTIME_DIR": user.runtime_dir,
        })
