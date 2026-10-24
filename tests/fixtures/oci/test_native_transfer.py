"""Synthetic artifact/cache proofs; no image producer or native VM runs here."""

from pathlib import Path

import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.leases import active_path, release_path, retain_generation
from capsem_builder.cache.paths import CachePaths

from tests.fixtures.oci import native_transfer

ROOT = Path(__file__).resolve().parents[3]


def paths(tmp_path):
    return CachePaths(repository_root=tmp_path, policy=load_policy(ROOT))


def test_destination_is_leased_before_copy_and_through_consumer(tmp_path):
    cache = paths(tmp_path)
    key = "native-transfer-synthetic"
    lease = cache.stage("ironbank") / ("." + key + ".lock")
    with native_transfer.destination(cache, key, 4096) as target:
        assert active_path(lease) and target.is_dir()
        (target / "payload").write_bytes(b"original")
        native_transfer.capacity(cache, "ironbank")
        assert active_path(lease)
    assert not active_path(lease)
    assert (target / "payload").read_bytes() == b"original"


@pytest.mark.parametrize("fault", ["size", "count", "symlink", "consumer"])
def test_retention_refuses_budget_alias_and_consumer_failure_without_deletion(tmp_path, fault):
    cache = paths(tmp_path)
    stage = cache.stage("ironbank")
    stage.mkdir(parents=True)
    key = "native-transfer-synthetic"
    if fault == "count":
        for number in range(cache.policy.stages["ironbank"].maximum_count):
            (stage / str(number)).mkdir()
    if fault == "symlink":
        outside = tmp_path / "preserved"
        outside.mkdir()
        (stage / key).symlink_to(outside, target_is_directory=True)
    required = cache.policy.stages["ironbank"].max_size_bytes + 1 if fault == "size" else 4096
    with pytest.raises(ValueError), native_transfer.destination(cache, key, required):
        raise ValueError("consumer failed")
    assert not active_path(stage / ("." + key + ".lock"))
    if fault == "symlink":
        assert outside.is_dir()


def test_other_real_generation_lease_is_preserved_by_owned_release(tmp_path):
    cache = paths(tmp_path)
    retain_generation(cache, "ironbank", "foreign")
    lease = cache.stage("ironbank") / ".foreign.lock"
    try:
        with native_transfer.destination(cache, "native-transfer-own", 4096):
            assert active_path(lease)
        assert active_path(lease)
    finally:
        release_path(lease)
