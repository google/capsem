"""Host-owned workload resource and identity-map validation."""

LOWEST_MAPPED_ID = 65536

def checked_id_map(id_map):
    """The host's map, refused unless container root and every id after it
    land at or above LOWEST_MAPPED_ID."""
    if (
        not isinstance(id_map, dict)
        or set(id_map) != {"containerID", "hostID", "size"}
        or not all(type(value) is int for value in id_map.values())
        or id_map["containerID"] != 0
        or id_map["hostID"] < LOWEST_MAPPED_ID
        or not 0 < id_map["size"] <= 65536
    ):
        raise ValueError(f"refusing id map {id_map!r}: container root must map above the VM's own ids")
    return id_map


def shm_bytes(resources):
    """POSIX shared memory for the workload: a quarter of its memory, at
    least 64 MiB and at most 1 GiB. Chromium renders into it and draws blank
    windows below about 256 MiB. Its pages are charged to the workload's
    cgroup like any other memory, so the size widens no limit."""
    return max(64 * 1024**2, min(1024**3, resources["memory_bytes"] // 4))


def checked_resources(resources):
    """The host's sizing, refused unless every limit is a positive integer."""
    if (
        not isinstance(resources, dict)
        or set(resources) != {"memory_bytes", "cpu_millis", "pids"}
        or not all(type(value) is int and value > 0 for value in resources.values())
    ):
        raise ValueError(f"refusing workload resources {resources!r}")
    return resources
