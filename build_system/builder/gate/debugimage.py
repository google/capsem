"""Build capsem-debug, the test-tooling image, into the shared fixture cache.

The maintainer's half of `[functional.debug_image]`: build the image for this
host's platform as an OCI layout, keyed by its manifest digest, in the cache
stage the kingslanding prefetch reads. The suites then serve exactly that
digest, and the command prints the line to pin in `config/gate.toml`.
Publishing the same bytes is a separate, manual step (see
tests/fixtures/oci/README.md); nothing here pushes.
"""

from __future__ import annotations

from .actions import Script
from .command import GateCommand
from .execution import Kind, Needs, Speed, step
from .plan import Plan


class DebugImageCommand(
    GateCommand,
    name="debug-image",
    help="build the capsem-debug test image into the fixture cache and print its digest pin",
):
    # It writes the shared cache stage and drives the Docker daemon a gate run
    # may be using.
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        plan.add(
            step(
                "debug-image.build",
                Script(
                    config,
                    config.functional.debug_image.script,
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
