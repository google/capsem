"""Image sessions: the default fixture for anything a user's workload does.

`image_session` boots a named session of a pinned image (the reference image,
the official `dev`, unless a test needs capsem-debug's tooling) and yields its
id once the workload runs. It is admitted the way an administrator would grant
one image: its repository as a source, that repository at that digest as the
one image admitted -- never the whole test registry.

`workload_exec` is how a test acts inside it. Every call proves where it ran:
a command meant for the workload that lands in the VM -- an omitted target
falling back, a workload that died -- fails the test instead of passing on
VM-side state. A bare-VM test says so explicitly with `target="vm"`.
"""

from __future__ import annotations

import contextlib
import time
from collections.abc import Iterator, Mapping
from pathlib import Path

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import exec_output_text
from tests.fixtures.oci.pinned_image import PinnedImage, reference_image
from tests.fixtures.oci.registry import grant_exact, layout_registry

#: Where the image's workload starts, and where the session's workspace -- the
#: files API's root -- appears inside it.
WORKSPACE = "/workspace"
#: Omit `target` from the request: the default a user's `capsem exec` takes.
DEFAULT_TARGET = None
#: First line of every workload exec: the uid map of the shell that ran it.
_PROBE = "CAPSEM-UID-MAP"
#: The VM's own namespace maps every id to itself; the workload's maps
#: container root to a host id above it.
_VM_UID_MAP = ("0", "0", "4294967295")


class NotInWorkload(AssertionError):
    """A command meant for the workload ran in the VM."""


@contextlib.contextmanager
def image_session(
    service,
    workdir: Path,
    name: str,
    *,
    image: PinnedImage = reference_image,
    layout: Path | None = None,
    client=None,
    env: Mapping[str, str] | None = None,
    ram_mb: int = DEFAULT_RAM_MB,
    cpus: int = DEFAULT_CPUS,
    timeout: float = 300,
) -> Iterator[str]:
    """A named session whose workload is the pinned `image`; yields its id
    once the workload runs, and deletes it afterwards.

    A missing layout fails here, never skips: `image.ready()` raises when the
    prefetch did not stage it. `layout` names one staged elsewhere (an
    installed proof's inputs), verified against the same pin; `client`
    defaults to the service's own socket (the gateway's TCP client drives it
    the same way).
    """
    if layout is None:
        layout = image.ready()
    else:
        image.verify(layout, image.pinned(), image.platform_name())
    with session_of(
        service,
        workdir,
        name,
        layout=layout,
        image_name=image.settings().name,
        client=client,
        env=env,
        ram_mb=ram_mb,
        cpus=cpus,
        timeout=timeout,
    ) as vm_id:
        yield vm_id


@contextlib.contextmanager
def session_of(
    service,
    workdir: Path,
    name: str,
    *,
    layout: Path,
    image_name: str,
    client=None,
    env: Mapping[str, str] | None = None,
    ram_mb: int = DEFAULT_RAM_MB,
    cpus: int = DEFAULT_CPUS,
    timeout: float = 300,
) -> Iterator[str]:
    """A named session of the image in `layout`, served by its own digest from
    a loopback registry and admitted as exactly that image. The registry stays
    up for the session, so a restart or fork that pulls again finds it."""
    workdir.mkdir(parents=True, exist_ok=True)
    client = client or service.client()
    with layout_registry(workdir, layout, image_name) as (reference, certificate, _):
        grant_exact(service.home_dir, reference)
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
            wait_running(client, vm_id, timeout=timeout)
            yield vm_id
        finally:
            with contextlib.suppress(Exception):
                client.delete(f"/vms/{vm_id}/delete", timeout=60)


def wait_running(client, vm_id: str, *, timeout: float = 300) -> None:
    """Until the session's workload runs; a failed or exited one fails at once."""
    deadline = time.monotonic() + timeout
    state = None
    while time.monotonic() < deadline:
        container = client.get(f"/vms/{vm_id}/container") or {}
        state = container.get("state")
        if state == "running":
            return
        assert state not in {"failed", "exited"}, container
        time.sleep(0.25)
    raise AssertionError(f"the workload of {vm_id} never ran: {state}")


def workload_exec(
    client,
    vm_id: str,
    command: str,
    *,
    target: str | None = "workload",
    timeout: int = 60,
) -> dict:
    """Run `command` in the session's workload and return the exec result, its
    stdout without the probe line. `target` is `"workload"` or `DEFAULT_TARGET`
    (omitted, as a user's exec sends it); both must land in the workload.
    """
    if target not in ("workload", DEFAULT_TARGET):
        raise ValueError(
            "workload_exec runs in the workload; use target='vm' explicitly elsewhere"
        )
    body: dict = {
        "command": f"printf '{_PROBE} %s\\n' \"$(head -1 /proc/self/uid_map)\"; {command}",
        "timeout_secs": timeout,
    }
    if target is not None:
        body["target"] = target
    result = client.post(f"/vms/{vm_id}/exec", body, timeout=timeout + 10)
    assert isinstance(result, dict), result
    stdout = exec_output_text(result)
    first, _, rest = stdout.partition("\n")
    if not first.startswith(_PROBE):
        raise NotInWorkload(
            f"the exec never reached a shell that reported its uid map: {result}"
        )
    if tuple(first.removeprefix(_PROBE).split()) == _VM_UID_MAP:
        raise NotInWorkload(f"{command!r} ran in the VM, not the workload: {first}")
    return {**result, "stdout_text": rest}
