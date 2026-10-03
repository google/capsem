"""The workload's resource limits bind under rootless runc.

runc runs rootless as uid 0, which can leave cgroup limits silently
unapplied. The workload's memory, processes and CPU are sized from the VM
minus the runtime's reserve (capsem-core `workload_resources`); a 1 GB VM
leaves the workload 640 MiB. The probe exceeds the memory and process limits
from inside the workload and must be stopped by them, not by the VM: the
shell that ran the hogs keeps going and reports.
"""

import subprocess

import pytest

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import command, environment, service

__all__ = ["service"]

pytestmark = pytest.mark.integration

PROBE = (
    # 900 MB held in one shell variable: past the 640 MiB limit, under the VM.
    '( hog=$(head -c 900000000 /dev/zero | tr "\\0" a); echo "HOG survived ${#hog}" ); echo "MEMORY $?"; '
    # More processes than the 4096 the workload may hold. dash exits when a
    # fork fails, so the forking runs in a subshell that may die; its sleeps
    # live on, and the parent counts every process in the workload with a
    # glob, because at the limit it cannot fork either.
    '( i=0; while [ $i -lt 4200 ]; do sleep 60 & i=$((i + 1)); done ) 2>/dev/null; '
    'set -- /proc/[0-9]*; echo "PIDS $#"; kill -9 -1 2>/dev/null; '
    'echo "ALIVE"'
)


def test_memory_and_process_limits_bind_inside_the_workload(service, tmp_path):
    with registry(tmp_path) as (reference, certificate, _):
        result = subprocess.run(
            [*command(service, reference, certificate, "--ram", "1", "--cpu", "1"), "/bin/sh", "-c", PROBE],
            env=environment(service),
            capture_output=True,
            timeout=300,
            check=False,
        )
    (tmp_path / "probe.stdout").write_bytes(result.stdout)
    (tmp_path / "probe.stderr").write_bytes(result.stderr)
    output = result.stdout.decode(errors="replace")
    words = output.split()
    assert "ALIVE" in output, result.stderr.decode(errors="replace") + output
    assert "HOG survived" not in output, output
    # The OOM killer took the hog's subshell (SIGKILL: 128 + 9).
    assert words[words.index("MEMORY") + 1] == "137", output
    # The loop got far, and the limit stopped it: never more than 4096.
    assert 1000 < int(words[words.index("PIDS") + 1]) <= 4096, output
