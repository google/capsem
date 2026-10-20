"""Build a pinned suite image into the shared fixture cache.

The maintainer's half of `[functional.debug_image]` (capsem-debug, the
test-tooling image) and `[functional.reference_image]` (the official `dev`
image every runtime test boots): build the image for this host's platform as
an OCI layout, keyed by its manifest digest, in the cache stage the
kingslanding prefetch reads. The suites then serve exactly that digest, and the
command prints the line to pin in `config/gate.toml`. Publishing the same bytes
is a separate, manual step (see tests/fixtures/oci/README.md); nothing here
pushes.
"""

from __future__ import annotations

from .actions import Script
from .command import GateCommand
from .config import GateConfig
from .execution import Kind, Needs, Speed, step
from .plan import Plan


def _build(name: str, config: GateConfig, section: str) -> Plan:
    """The one build step both commands plan, for their `[functional.<section>]`."""
    plan = Plan(name)
    plan.add(
        step(
            f"{name}.build",
            Script(
                config,
                getattr(config.functional, section).script,
                section,
                "build",
                "--platform",
                config.host_arch().docker_platform,
                outside_sandbox=True,
            ),
            contends=(config.exclusive("docker_daemon"),),
            kind=Kind.COMPILE,
            needs=frozenset({Needs.DISK, Needs.DOCKER, Needs.NETWORK}),
            speed=Speed.SLOW,
        )
    )
    return plan


class DebugImageCommand(
    GateCommand,
    name="debug-image",
    help="build the capsem-debug test image into the fixture cache and print its digest pin",
):
    # Both write the shared cache stage and drive the Docker daemon a gate run
    # may be using.
    exclusive = True

    def plan(self) -> Plan:
        return _build(self.name, self._config, "debug_image")


class ReferenceImageCommand(
    GateCommand,
    name="reference-image",
    help="build the reference (official dev) image into the fixture cache and print its digest pin",
):
    exclusive = True

    def plan(self) -> Plan:
        return _build(self.name, self._config, "reference_image")
