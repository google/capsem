"""Real workload file producers and untimed policy/ledger completion proof."""
from __future__ import annotations

import json
import shlex

from helpers.image_session import WORKSPACE, workload_exec
from helpers.settings_policy import apply_settings_rule

from .route_fixtures import corpus_config

RULE = "route_binary_export"


def seed_binary_files(service, vm_id: str | None) -> None:
    assert vm_id is not None, "binary fixture needs a real live owner"
    sizes = corpus_config()["file_sizes"]
    names = [f"route-{size}.bin" for size in sizes]
    # Only these isolated synthetic files are allowed. The real owner still
    # evaluates policy and commits file/security history for every download.
    apply_settings_rule(service, RULE, action="allow", detection_level="informational",
                        match="file.export.name in " + json.dumps(names), reason="Isolated benchmark binary export corpus.")
    source = f"from pathlib import Path; sizes={sizes!r}; root=Path({WORKSPACE!r}); " + "; ".join(
        f"(root / {name!r}).write_bytes(bytes(range(256)) * ({size} // 256))" for name, size in zip(names, sizes, strict=True)
    ) + "; print('CAPSEM-BINARY-FIXTURE')"
    result = workload_exec(service.client(), vm_id, "python3 -c " + shlex.quote(source))
    assert result["exit_code"] == 0 and result["stdout_text"].strip() == "CAPSEM-BINARY-FIXTURE"


def finish_binary_files(service, vm_id: str | None, expected: int) -> None:
    # Stopping drains the actual producer before public-ledger assertions;
    # sleeps/poll caches are not a read-after-write barrier. Session stays in
    # the registry until image_session's owning context deletes it normally.
    assert vm_id is not None
    client = service.client()
    stopped = client.post(f"/vms/{vm_id}/stop", {}, timeout=60)
    assert stopped["success"] is True and stopped["persistent"] is True
    status = client.get(f"/vms/{vm_id}/enforcement/status")
    records = [row for row in status["by_rule"] if row["rule_id"] == "profiles.rules." + RULE]
    assert len(records) == 1 and records[0]["count"] == expected and records[0]["rule_action"] == "allow", status
    files = client.get(f"/vms/{vm_id}/info")["files"]
    exports = [row for row in files["actions"] if row["action"] == "export"]
    assert len(exports) == 1 and exports[0]["count"] == expected, files


def expected_file_exports(cases, *, samples: int, load_requests: int, concurrency: int, warmup: int, targets: int, override: int | None) -> int:
    total = 0
    for case in cases:
        if case.response != "binary":
            continue
        count, requests, workers = case.counts(samples, load_requests, concurrency, override)
        # Preflight + warm setup + warm/cold samples + concurrent worker setup/load.
        total += targets * (1 + warmup + 2 * count + min(requests, workers) + requests)
    return total
