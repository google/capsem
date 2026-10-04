"""Sessions of capsem-debug, the test-tooling image.

The VM runtime carries no test tooling. A test that needs a test runner, a
network tool, a package manager, a model SDK or an agent CLI opens one of
these sessions and execs into its workload; `"target": "vm"` still reaches
the VM. The image is served by the digest `config/gate.toml` pins, from a
hermetic registry on loopback (tests/fixtures/oci/README.md).
"""

from __future__ import annotations

import contextlib
import time
from collections.abc import Iterator, Mapping
from pathlib import Path

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from tests.fixtures.oci import debug_image
from tests.fixtures.oci.registry import grant_image, layout_registry

#: Where the image's workload starts, and where the session's workspace -- the
#: files API's root -- appears inside it.
WORKSPACE = "/workspace"


@contextlib.contextmanager
def debug_session(
    service,
    workdir: Path,
    name: str,
    *,
    env: Mapping[str, str] | None = None,
    ram_mb: int = DEFAULT_RAM_MB,
    cpus: int = DEFAULT_CPUS,
    timeout: float = 300,
) -> Iterator[str]:
    """A named session whose workload is capsem-debug; yields its id once the
    workload runs, and deletes it afterwards.

    The registry stays up for the session, so a restart or fork that pulls
    again finds the same digest.
    """
    workdir.mkdir(parents=True, exist_ok=True)
    client = service.client()
    with layout_registry(workdir, debug_image.ready(), debug_image.settings().name) as (
        reference,
        certificate,
        _,
    ):
        grant_image(service.home_dir, reference)
        created = client.post(
            "/vms/create",
            {
                "name": name,
                "ram_mb": ram_mb,
                "cpus": cpus,
                "container": {
                    "image": reference,
                    "env": dict(env or {}),
                    "registry": {"ca_pem": certificate.read_text()},
                },
            },
            timeout=timeout,
        )
        assert created is not None and created.get("id"), created
        vm_id = created["id"]
        try:
            deadline = time.monotonic() + timeout
            state = None
            while time.monotonic() < deadline:
                state = (client.get(f"/vms/{vm_id}/container") or {}).get("state")
                if state == "running":
                    break
                assert state not in {"failed", "exited"}, client.get(f"/vms/{vm_id}/container")
                time.sleep(0.25)
            assert state == "running", f"capsem-debug workload never ran: {state}"
            yield vm_id
        finally:
            with contextlib.suppress(Exception):
                client.delete(f"/vms/{vm_id}/delete", timeout=60)
