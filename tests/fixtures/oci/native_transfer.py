"""One artifact generation, retained by the existing native handoff owner.

No uploader or cache policy lives here. Inventory and lease mutation use the
canonical cache owner; normal release only closes the held generation FD.
"""

from __future__ import annotations

import re
from contextlib import contextmanager

from capsem_builder.cache.inventory import scan_inventory
from capsem_builder.cache.leases import release_path, retain_generation


def capacity(paths, stage_id, additional_bytes=0, additional_entries=0):
    policy = paths.policy.stages[stage_id]
    (current,) = scan_inventory(paths, paths.policy, stage_ids=frozenset({stage_id})).stages
    count = sum(not entry.lease_only for entry in current.entries)
    if (
        additional_bytes < 0
        or additional_entries < 0
        or current.budget_bytes + additional_bytes > policy.max_size_bytes
        or (policy.maximum_count is not None and count + additional_entries > policy.maximum_count)
    ):
        raise ValueError("native artifact exceeds actual cache stage headroom")
    return current


@contextmanager
def destination(paths, key, required_bytes):
    if not re.fullmatch(r"native-transfer-[A-Za-z0-9][A-Za-z0-9._-]*", key):
        raise ValueError("native artifact generation is not fixed and contained")
    stage_id = "ironbank"
    policy = paths.policy.stages[stage_id]
    template = policy.lease_template
    if template is None or required_bytes <= 0:
        raise ValueError("native artifact requires a finite lease and byte bound")
    root = paths.stage(stage_id)
    target = root / key
    if target.exists() or target.is_symlink():
        raise ValueError("native artifact generation already exists")
    capacity(paths, stage_id, required_bytes, 1)
    retain_generation(paths, stage_id, key)
    try:
        target.mkdir(parents=True, exist_ok=False)
        capacity(paths, stage_id, required_bytes)
        yield target
        capacity(paths, stage_id)
    finally:
        release_path(root / template.format(key=key))
