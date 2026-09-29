"""A new source-keyed bytecode generation brings its stage back under contract.

`python-pycache` measured 5.4 GB and later 2.8 GB against a 2 GiB maximum and
a 20-generation count. Every gate launch and every bounded command selects a
generation keyed by the checkout's Python sources, so each edit in any
worktree adds one (24-207 MB each, 29 on disk at once), while the only
routine prune ran inside the package and install rails of a complete gate.
Nothing bounded the stage between complete runs, so it grew with agent
activity. The launcher that adds a generation now enforces the stage, with
its own generation leased first.
"""

from __future__ import annotations

import fcntl
import os
import subprocess
import sys
import time
from pathlib import Path

from capsem_builder.cache.config import load_policy

ROOT = Path(__file__).resolve().parents[3]
AUTHORITY = load_policy(ROOT).authority_environment
GENERATION_BYTES = 4096


def _source(tmp_path: Path) -> Path:
    source = tmp_path / "checkout"
    (source / "config").mkdir(parents=True)
    policy = (ROOT / "config/cache.toml").read_text(encoding="utf-8")
    scratch_root = tmp_path / "scratch/capsem-tests"
    policy = policy.replace('path = "/var/tmp/capsem-tests"', f'path = "{scratch_root}"')
    # Small enough to cross with a few KiB: three generations are over max.
    stage = '[stages.python-pycache]\ndescription = "Source-keyed Python bytecode environments used by gate commands."\nscope = "disk"\npath = "tools/python/pycache"\nwarm_size_bytes = 1073741824 # 1 GiB\nmax_size_bytes = 2147483648 # 2 GiB'
    assert stage in policy
    policy = policy.replace(
        stage,
        stage.replace("1073741824 # 1 GiB", str(GENERATION_BYTES)).replace(
            "2147483648 # 2 GiB", str(int(2.5 * GENERATION_BYTES))
        ),
    )
    (source / "config/cache.toml").write_text(policy, encoding="utf-8")
    (source / "config/gate.toml").write_bytes((ROOT / "config/gate.toml").read_bytes())
    (source / "module.py").write_text("VALUE = 1\n", encoding="utf-8")
    return source


def _generation(stage: Path, name: str, age_seconds: int) -> Path:
    generation = stage / f"cpython-312-{name}"
    (generation / "tree").mkdir(parents=True)
    (generation / "tree/module.cpython-312.pyc").write_bytes(b"x" * GENERATION_BYTES)
    stamp = time.time() - age_seconds
    for path in (generation / "tree/module.cpython-312.pyc", generation / "tree", generation):
        os.utime(path, (stamp, stamp))
    return generation


def _launch(source: Path, authority: Path) -> subprocess.CompletedProcess[str]:
    probe = f"""
from pathlib import Path
from capsem_builder import gatelaunch
environment = gatelaunch.isolated_environment(Path({str(source)!r}), authority=Path({str(authority)!r}))
print(environment[gatelaunch.PYCACHE])
"""
    return subprocess.run(
        [sys.executable, "-c", probe],
        env={k: v for k, v in os.environ.items() if k != AUTHORITY},
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )


def test_a_new_generation_prunes_idle_ones_and_keeps_itself_and_leased_ones(
    tmp_path: Path,
) -> None:
    source = _source(tmp_path)
    authority = tmp_path / "authority"
    stage = authority / "cache/tools/python/pycache"
    oldest = _generation(stage, "oldest", 3000)
    older = _generation(stage, "older", 2000)
    leased = _generation(stage, "leased", 4000)
    lease = stage / f".{leased.name}.lock"
    lease.touch()

    with lease.open("rb") as descriptor:
        fcntl.flock(descriptor, fcntl.LOCK_SH)
        result = _launch(source, authority)

    assert result.returncode == 0, result.stderr
    created = Path(result.stdout.strip().splitlines()[-1])
    assert created.parent == stage and created.is_dir()
    assert leased.is_dir(), "a generation another process holds was pruned"
    assert not oldest.exists() and not older.exists(), "idle generations outlived the bound"


def test_an_existing_generation_does_not_pay_for_enforcement(tmp_path: Path) -> None:
    source = _source(tmp_path)
    authority = tmp_path / "authority"
    first = _launch(source, authority)
    assert first.returncode == 0, first.stderr
    stage = authority / "cache/tools/python/pycache"
    idle = _generation(stage, "idle", 5000)
    _generation(stage, "idle-too", 6000)

    again = _launch(source, authority)

    assert again.returncode == 0, again.stderr
    assert idle.is_dir(), "a warm launch ran retention it did not need"
