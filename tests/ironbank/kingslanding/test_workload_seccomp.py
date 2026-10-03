"""The workload's syscall filter, observed from inside a real workload.

`capsem run --image` launches the production launcher, so the workload runs
under the filter capsem-core resolves (deny by default, moby's allowlist,
no namespace creation, no mount). The probe is the image's own shell: a
process entered from outside with nsenter would not carry the filter.

vsock, keyctl and netfilter netlink need a probe program the fixture image
does not have; the filter's content is proven by capsem-core's
`container::seccomp` tests and the full in-workload probe arrives with the
capsem-debug image.
"""

import subprocess

import pytest

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import command, environment, service

__all__ = ["service"]

pytestmark = pytest.mark.integration

PROBE = (
    "for probe in 'unshare -U true' 'unshare -n true' 'mount -t tmpfs none /mnt' 'nsenter -t 1 -m true'; do "
    '  if $probe 2>/dev/null; then echo "ALLOWED $probe"; else echo "DENIED $probe"; fi; '
    "done; "
    'echo "ORDINARY $(echo works)"'
)


def test_the_workload_cannot_create_namespaces_or_mount(service, tmp_path):
    with registry(tmp_path) as (reference, certificate, _):
        result = subprocess.run(
            [*command(service, reference, certificate), "/bin/sh", "-c", PROBE],
            env=environment(service),
            capture_output=True,
            timeout=120,
            check=False,
        )
    (tmp_path / "probe.stdout").write_bytes(result.stdout)
    (tmp_path / "probe.stderr").write_bytes(result.stderr)
    output = result.stdout.decode(errors="replace")
    assert result.returncode == 0, result.stderr.decode(errors="replace")
    assert "ORDINARY works" in output, output
    assert "ALLOWED" not in output, output
    assert output.count("DENIED") == 4, output
