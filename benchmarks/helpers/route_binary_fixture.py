"""Real workload file producers and untimed policy/ledger completion proof."""
from __future__ import annotations

import re
import shlex

from helpers.image_session import WORKSPACE, workload_exec
from helpers.settings_policy import apply_settings_rule, file_name_condition

from .route_fixtures import corpus_config

RULE = "route_binary_export"


def seed_binary_files(service, vm_id: str | None) -> None:
    assert vm_id is not None, "binary fixture needs a real live owner"
    sizes = corpus_config()["file_sizes"]
    names = [f"route-{size}.bin" for size in sizes]
    # Only these isolated synthetic files are allowed. The real owner still
    # evaluates policy and commits file/security history for every download.
    apply_settings_rule(service, RULE, action="allow", detection_level="informational",
                        match=file_name_condition("export", names), reason="Isolated benchmark binary export corpus.")
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


def verify_import_evidence(read,vm_id,expected,rule,limit):
    """Public ledger-owner reads after producer drain; never open its DB here."""
    status = read(f'/vms/{vm_id}/enforcement/status')
    rules = [row for row in status['by_rule'] if row['rule_id'] == 'profiles.rules.'+rule]
    assert len(rules) == 1 and rules[0]['count'] == len(expected) and rules[0]['rule_action'] == 'allow'
    files = read(f'/vms/{vm_id}/info')['files']
    imports = [row for row in files['actions'] if row['action'] == 'import']
    # Image startup may import other paths; the exact rule count and selected
    # original event IDs below isolate this corpus rather than guessing it.
    assert len(imports) == 1 and imports[0]['count'] >= len(expected)
    selected = [row for row in read(f'/vms/{vm_id}/stats/detail')['file_events'] if row['action'] == 'import' and row['path'] in expected]
    assert len(selected) == len(expected), 'import events missing or duplicated'
    assert {row['path'] for row in selected} == set(expected)
    assert len({row['event_id'] for row in selected}) == len(expected)
    for row in selected:
        assert row['size'] == expected[row['path']] and row['credential_ref'] is None
        assert re.fullmatch(r'[0-9a-f]{12}',row['event_id']), 'import event identity invalid'
    timeline = read(f'/vms/{vm_id}/timeline?layers=fs&limit={limit}')['events']
    for name in expected:
        matched = [row for row in timeline if row['layer'] == 'fs' and row['summary'] == 'import '+name]
        assert len(matched) == 1 and matched[0]['ref'] is not None, 'import timeline event missing or duplicated'
