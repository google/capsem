"""Real foreign OCI leases survive retention, including their own lock inode."""

import select
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.cache.api import CacheOperation, CacheRequest
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.inventory import scan_inventory
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.registry import CacheRegistry

from .test_inventory import policy

ROOT = Path(__file__).resolve().parents[3]


@pytest.mark.parametrize("patterns", [None, ("*",), (".*", "*")])
def test_real_image_lease_and_payload_survive_prune_and_clean_until_foreign_owner_exits(tmp_path, patterns):
    stage = load_policy(ROOT).stages["test-images"].model_copy(update={
        "warm_size_bytes": 0, "max_size_bytes": 1, "maximum_count": 1,
    })
    if patterns is not None:
        stage = stage.model_copy(update={"managed_globs": patterns})
    configured = policy().model_copy(update={"stages": {"test-images": stage}})
    paths = CachePaths(repository_root=tmp_path, policy=configured)
    generation = paths.stage("test-images") / "dev-exact-digest"
    generation.mkdir(parents=True)
    (generation / "payload").write_bytes(b"leased OCI bytes")
    assert stage.lease_template is not None
    lease = generation.parent / stage.lease_template.format(key=generation.name)
    holder = (
        "import fcntl,os,signal,sys; signal.alarm(15); "
        "f=open(sys.argv[1],'a+b'); fcntl.flock(f,fcntl.LOCK_SH); "
        "print(os.fstat(f.fileno()).st_ino,flush=True); sys.stdin.buffer.read(1)"
    )
    owner = subprocess.Popen([sys.executable, "-c", holder, str(lease)],
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    registry = CacheRegistry(paths, configured)
    try:
        assert owner.stdout is not None
        assert select.select([owner.stdout], [], [], 10)[0], "foreign owner did not acquire its real lease"
        inode = int(owner.stdout.readline())
        for operation in (CacheOperation.PRUNE, CacheOperation.CLEAN, CacheOperation.ENFORCE):
            registry.mutate(CacheRequest(operation=operation, cache_id="test-images", apply=True,
                                         reason="foreign OCI lease regression", checkout=tmp_path))
            assert lease.exists(), "retention unlinked a held lease as though it were a payload"
            assert lease.stat().st_ino == inode
            assert (generation / "payload").read_bytes() == b"leased OCI bytes"
            entries = scan_inventory(paths, configured).stages[0].entries
            assert len(entries) == 1 and entries[0].protected
            assert entries[0].member_paths == (Path(lease.name),)
            assert not list(generation.parent.glob("..*.lock.lock"))
    finally:
        try:
            owner.communicate(b"x", timeout=5)
        except subprocess.TimeoutExpired:
            owner.kill()
            owner.communicate(timeout=5)
    assert owner.returncode == 0
    registry.mutate(CacheRequest(operation=CacheOperation.CLEAN, cache_id="test-images", apply=True,
                                reason="released OCI generation is reclaimable", checkout=tmp_path))
    assert not generation.exists() and not lease.exists()
