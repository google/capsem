"""The workload runs in a user namespace and still owns its workspace.

Container ids 0-65535 are VM ids 100000-165535 (capsem-core
`WORKLOAD_ID_MAP`), so container root is no uid the VM trusts. The workspace
share reports every entry as VM uid 0; the launcher mounts it through the same
map, so inside the container it is root's and the archive tools that chown --
`tar -x`, `cp -a` -- work there as they would on any root filesystem.
"""

import subprocess

import pytest

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import command, environment, service

__all__ = ["service"]

pytestmark = pytest.mark.integration

PROBE = (
    'echo "MAP $(cat /proc/self/uid_map)"; '
    'echo "OWNER $(stat -c %u /workspace)"; '
    "cd /workspace && rm -rf userns-probe && mkdir userns-probe && cd userns-probe && "
    "echo x > file && chown 1:1 file && echo CHOWN ok; "
    "mkdir -p /tmp/src && echo y > /tmp/src/a && chmod 640 /tmp/src/a && "
    'cp -a /tmp/src copied && echo "CPA $(stat -c %a copied/a)"; '
    "tar -C /tmp -cf /tmp/src.tar src && tar -xf /tmp/src.tar && echo TAR ok"
)


def test_the_workload_is_an_unprivileged_root_that_owns_its_workspace(service, tmp_path):
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
    words = output.split()
    assert result.returncode == 0, result.stderr.decode(errors="replace") + output
    assert words[words.index("MAP") + 1 : words.index("MAP") + 4] == ["0", "100000", "65536"], output
    assert words[words.index("OWNER") + 1] == "0", output
    assert "CHOWN ok" in output, output
    assert words[words.index("CPA") + 1] == "640", output
    assert "TAR ok" in output, output
