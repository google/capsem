"""An image-policy update imports only scratch through the supported clone path.

All home contents here are synthetic. No existing user's auth volume is copied.
The source remains unchanged; the replacement image selects the workspace alias.
"""

import json

import pytest
from helpers.service import exec_output_text

from tests.fixtures.oci.registry import FIXTURES, registry
from tests.ironbank.kingslanding.test_run import created, service, wait_for
from tests.ironbank.kingslanding.test_workload_exec import run

__all__ = ["service"]
pytestmark = pytest.mark.integration
HOME = "/home/capsem"
SCRATCH = f"{HOME}/.gemini/antigravity/scratch"
POLICY = "org.capsem.workspace.policy"


def test_new_image_clone_imports_scratch_without_changing_original_home(service, tmp_path):
    client = service.client()
    original = json.loads((FIXTURES / "redis-config.json").read_text())
    base = {
        **original,
        "Entrypoint": ["/bin/sh", "-c"],
        "Cmd": ["echo workspace-fixture-ready; exec sleep 3600"],
        "WorkingDir": "/workspace",
        "Volumes": {HOME: {}},
        "User": "0:0",
    }
    replacement = {**base, "User": "1000:1000", "Labels": {POLICY: "antigravity-scratch-v1"}}
    (tmp_path / "old").mkdir()
    (tmp_path / "new").mkdir()
    with (
        registry(tmp_path / "old", image_config=base) as (old, old_ca, _),
        created(service, tmp_path, old, old_ca, "scratch-original", ready="workspace-fixture-ready") as source,
    ):
        source_id = source["id"]
        run(client, source_id, f"set -e; mkdir -p {SCRATCH}; printf scratch-original > {SCRATCH}/poem.md; "
            f"printf synthetic-private-sibling > {HOME}/private-sibling; "
            f"chown -R 1000:1000 {HOME}; printf workspace-wins > /workspace/poem.md")
        with (
            registry(tmp_path / "new", image_config=replacement) as (new, new_ca, _),
            created(service, tmp_path, new, new_ca, "scratch-copy", "--from", "scratch-original",
                    ready="workspace-fixture-ready") as copy,
        ):
            copy_id = copy["id"]
            assert run(client, copy_id, "id -u").strip() == "1000"
            assert run(client, copy_id, "stat -c %u /workspace").strip() == "1000"
            assert run(client, copy_id, "cat /workspace/poem.md") == "workspace-wins"
            # Preserve the first read before stat can refresh caller-owned
            # VirtioFS inode metadata. Diagnostics must not make the read pass.
            imported_read = client.post(f"/vms/{copy_id}/exec", {
                "command": "cat /workspace/poem.md.imported-1", "timeout_secs": 5,
            }, timeout=20)
            imported_owner = run(
                client, copy_id,
                "stat -c '%u:%g:%a' /workspace/poem.md.imported-1",
            ).strip()
            assert imported_read["exit_code"] == 0, (imported_owner, imported_read)
            assert imported_owner == "1000:1000:600", imported_owner
            assert exec_output_text(imported_read) == "scratch-original"
            assert run(client, copy_id, f"cat {HOME}/private-sibling") == "synthetic-private-sibling"
            assert run(client, copy_id, "test ! -e /workspace/private-sibling && echo isolated").strip() == "isolated"
            assert run(client, copy_id, f"test ! -r {SCRATCH}/.capsem-image/launch.py && echo masked").strip() == "masked"
            result = client.post(f"/vms/{copy_id}/exec", {
                "command": f"printf visible-event > {SCRATCH}/workspace-event.md", "timeout_secs": 5,
            }, timeout=20)
            assert result["exit_code"] == 0, result
            assert exec_output_text(result) == ""
            assert client.get_bytes(f"/vms/{copy_id}/files/content?path=workspace-event.md") == (200, b"visible-event")
            assert client.get_bytes(f"/vms/{copy_id}/files/content?path=poem.md.imported-1") == (200, b"scratch-original")
            wait_for(lambda: any("workspace-event.md" in event["path"] for event in
                client.get(f"/vms/{copy_id}/stats/detail")["file_events"]),
                "scratch write recorded through canonical workspace", timeout=20)
            # The source and original bytes are still usable throughout the clone.
            assert run(client, source_id, f"cat {SCRATCH}/poem.md") == "scratch-original"
            assert run(client, source_id, "cat /workspace/poem.md") == "workspace-wins"
            assert run(client, source_id, f"test ! -e {SCRATCH}/workspace-event.md && echo unchanged").strip() == "unchanged"
