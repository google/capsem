"""A fork of an image session is an image session, and so is `--from`.

The clone carries the workspace, which stages the pinned image, and the VM's
system overlay, which holds the image's volumes. The fork must also stay an
image session to the service: its status names the same digest, and an
untargeted exec enters its workload (user namespace, image user), not the
VM. `create --from SOURCE --image IMAGE` clones the same state but runs the
image it names, the only one launched, over the source's volume contents.
"""

import json

import pytest

from tests.fixtures.oci.registry import FIXTURES, registry
from tests.ironbank.kingslanding.test_run import created, service, wait_for
from tests.ironbank.kingslanding.test_workload_exec import IN_WORKLOAD, fields, run

__all__ = ["service"]

pytestmark = pytest.mark.integration

PROOF = "/data/fork-proof"


def container(client, vm_id):
    status, body = client.call_json("GET", f"/vms/{vm_id}/container")
    return body if status == 200 else None


def in_workload(client, vm_id):
    return fields(run(client, vm_id, f'{IN_WORKLOAD}; echo "DATA $(cat {PROOF})"'))


def test_a_fork_and_a_from_clone_stay_image_sessions_with_their_volume(service, tmp_path):
    client = service.client()
    with (
        registry(tmp_path) as (reference, certificate, _),
        created(service, tmp_path, reference, certificate, "fork-source") as source,
    ):
        run(client, source["id"], f"echo carried > {PROOF}")
        digest = container(client, source["id"])["digest"]
        fork_id = client.post(f"/vms/{source['id']}/fork", {"name": "fork-copy"})["id"]
        try:
            client.post(f"/vms/{fork_id}/resume", {}, timeout=120)
            wait_for(
                lambda: (container(client, fork_id) or {}).get("state") == "running",
                "the fork reports its workload running",
                timeout=120,
            )
            assert container(client, fork_id)["digest"] == digest
            inside = in_workload(client, fork_id)
            assert inside["MAP"].split() == ["0", "100000", "65536"], inside
            assert inside["DATA"] == "carried", inside
        finally:
            client.delete(f"/vms/{fork_id}/delete")

        with created(
            service, tmp_path, reference, certificate, "from-copy", "--from", "fork-source"
        ) as copy:
            status = container(client, copy["id"])
            assert status["state"] == "running", status
            assert status["image"] == reference, status
            inside = in_workload(client, copy["id"])
            assert inside["MAP"].split() == ["0", "100000", "65536"], inside
            assert inside["DATA"] == "carried", inside


LAYER_PROOF = "/var/layer-proof"


def test_a_session_that_changes_image_keeps_its_layer(service, tmp_path):
    """The workload's root is a copy-on-write layer the session owns, above
    whichever image it runs. `create --from SOURCE --image OTHER` swaps the
    image underneath; what the workload wrote to its root -- here outside any
    volume and outside the workspace -- is still there over the new image."""
    client = service.client()
    config = json.loads((FIXTURES / "redis-config.json").read_text())
    other = {**config, "Env": [*config["Env"], "CAPSEM_LAYER_TEST=other-image"]}
    (tmp_path / "first").mkdir()
    (tmp_path / "second").mkdir()
    with (
        registry(tmp_path / "first") as (first, first_certificate, _),
        created(service, tmp_path, first, first_certificate, "layer-source") as source,
    ):
        run(client, source["id"], f"echo kept > {LAYER_PROOF}")
        first_digest = container(client, source["id"])["digest"]
        with (
            registry(tmp_path / "second", image_config=other) as (second, second_certificate, _),
            created(
                service, tmp_path, second, second_certificate, "layer-copy", "--from", "layer-source"
            ) as copy,
        ):
            status = container(client, copy["id"])
            assert status["image"] == second and status["digest"] != first_digest, status
            inside = fields(run(client, copy["id"], f'{IN_WORKLOAD}; echo "DATA $(cat {LAYER_PROOF})"'))
            assert inside["MAP"].split() == ["0", "100000", "65536"], inside
            assert inside["DATA"] == "kept", inside
            env = fields(run(client, copy["id"], 'echo "DATA $CAPSEM_LAYER_TEST"'))
            assert env["DATA"] == "other-image", env
