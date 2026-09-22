#!/usr/bin/env python3
"""Ledger economics under a local model: bytes per request on disk and in RAM.

A sandboxed VM runs a loop for `CAPSEM_ECON_MINUTES` (default 30): fetch a page
from a fixed list of public sites, ask the host's Ollama model (through
capsem's egress, exactly as `vm_ollama.py` does) for a three-bullet summary,
repeat. Every five minutes the script samples session.db (+ WAL),
session.bodies, the ledger row counts, and the RSS of this VM's capsem-process
and of this run's capsem-service, then reports the per-request cost.

PASS when:
  - each request adds < 6 KB on disk (session.db + WAL + session.bodies),
    measured as a slope from the 10-minute mark to the last sample, so the
    empty schema's fixed floor (~470 KB) is not billed to the requests;
  - capsem-process RSS grows < 2 KB per request over the same window;
  - capsem-service RSS grows < 512 B per request over the same window (the
    service reads ledgers from disk, so it must not grow with traffic);
  - only capsem-process ever holds session.bodies open for writing;
  - the request count grows between every pair of samples, and the model was
    actually called. The measurements come from the ledger, not from exec
    output, so an exec that silently does nothing would otherwise "pass" with
    zero traffic; a flat line is a FAIL, never a pass.

It also prints total disk divided by request count (which includes the fixed
floor) and the compression ratio actually achieved: raw body bytes the index
holds against the size of session.bodies.

Usage (build first, then bound the run so no VM leaks):
    uv run --project build_system --frozen capsem-gate sign
    uv run --project build_system --frozen python tests/fixtures/oci/prepare_redis.py \
        --output cache/target/tests/redis-image --image redis
    python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 2700 \
        -- uv run --project build_system --frozen python tests/manual/ledger_economics.py

Env: CAPSEM_ECON_MODEL (default "gemma4"), CAPSEM_ECON_MINUTES (default 30),
CAPSEM_ECON_KEEP_DIR (copy the ledger files there before teardown, for offline replay).
"""

from __future__ import annotations

import contextlib
import itertools
import json
import os
import sqlite3
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path

# Only sys.path calls may sit above these first-party imports, or E402 fires;
# the imports cannot move up, they need the path set first.
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tests"))
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from helpers.body_archive import generation_path_for_db
from helpers.constants import CODE_PROFILE_ID
from helpers.service import ServiceInstance, vm_session_dir

from tests.fixtures.oci.registry import registry
from tests.manual.vm_ollama import boot, closing_ro, guest

os.environ.setdefault("CAPSEM_TRAY_HEADLESS", "1")

MODEL = os.environ.get("CAPSEM_ECON_MODEL", "gemma4")
MINUTES = int(os.environ.get("CAPSEM_ECON_MINUTES", "30"))
SAMPLE_EVERY_S = 300
KEEP_DIR = os.environ.get("CAPSEM_ECON_KEEP_DIR")
REDIS_FIXTURE = Path(__file__).resolve().parents[2] / "cache/target/tests/redis-image"
SITES = [
    "https://en.wikipedia.org/wiki/Special:Random",
    "https://news.ycombinator.com/",
    "https://docs.python.org/3/whatsnew/index.html",
    "https://doc.rust-lang.org/book/",
    "https://www.sqlite.org/whentouse.html",
]

# Runs in the guest's main namespace, where the :11434 redirect to the
# net-proxy applies. The closing line is the proof the loop really ran.
LOOP = """python3 - <<'PY'
import json, time, urllib.request
sites = {sites!r}
deadline = time.time() + {seconds}
i = fetched = answered = 0
while time.time() < deadline:
    url = sites[i % len(sites)]; i += 1
    try:
        html = urllib.request.urlopen(url, timeout=30).read()[:200000].decode("utf-8", "replace")
        fetched += 1
    except Exception as e:
        html = f"fetch failed: {{e}}"
    body = json.dumps({{"model": {model!r}, "stream": False,
        "prompt": "Summarise this page in three bullets:\\n" + html}}).encode()
    req = urllib.request.Request("http://localhost:11434/api/generate", data=body,
        headers={{"Content-Type": "application/json"}})
    try:
        urllib.request.urlopen(req, timeout=180).read()
        answered += 1
    except Exception as e:
        print("model failed:", e)
    time.sleep(20)
print(f"LOOP DONE iterations={{i}} fetched={{fetched}} answered={{answered}}")
PY"""


def host_has_model() -> bool:
    """Fail before booting anything if the host Ollama cannot serve MODEL."""
    try:
        with urllib.request.urlopen("http://127.0.0.1:11434/api/tags", timeout=5) as r:
            names = [m.get("name", "") for m in json.loads(r.read()).get("models", [])]
    except OSError as e:
        print(f"  [FAIL] host Ollama unreachable on :11434: {e}")
        return False
    if not any(n == MODEL or n.split(":")[0] == MODEL for n in names):
        print(f"  [FAIL] host Ollama has no {MODEL!r}; `ollama list` shows {names}")
        return False
    return True


def host_has_redis_fixture() -> bool:
    """Fail before starting the service when the pinned OCI fixture is absent."""
    required = (
        REDIS_FIXTURE / "redis-image.json",
        REDIS_FIXTURE / "redis-rootfs.tar.gz",
    )
    missing = [str(path) for path in required if not path.is_file()]
    if not missing:
        return True
    print(f"  [FAIL] Redis OCI fixture is missing: {', '.join(missing)}")
    print(
        "  run: uv run --project build_system --frozen python "
        "tests/fixtures/oci/prepare_redis.py "
        "--output cache/target/tests/redis-image --image redis"
    )
    return False


def rss_kb(pids: list[str]) -> int:
    total = 0
    for pid in pids:
        out = subprocess.run(
            ["ps", "-o", "rss=", "-p", pid], capture_output=True, text=True, check=False
        ).stdout.strip()
        total += int(out or 0)
    return total


def process_pids(vm_id: str) -> list[str]:
    return subprocess.run(
        ["pgrep", "-f", f"capsem-process.*--id {vm_id}"],
        capture_output=True,
        text=True,
        check=False,
    ).stdout.split()


def writers_of(path: Path) -> list[str]:
    """Commands holding `path` open for write (lsof access w or u)."""
    out = subprocess.run(
        ["lsof", "-F", "cfa", "--", str(path)],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    command, writers = "", []
    for line in out.splitlines():
        if line.startswith("c"):
            command = line[1:]
        elif line.startswith("a") and line[1:] in ("w", "u"):
            writers.append(command)
    return sorted(set(writers))


def _size(path: Path) -> int:
    return path.stat().st_size if path.exists() else 0


# Request/model counts, and the body archive from the block side and the index side.
LEDGER_SQL = """SELECT
    (SELECT COUNT(*) FROM net_events) AS reqs,
    (SELECT COUNT(*) FROM model_calls) AS models,
    (SELECT COUNT(*) FROM body_blocks) AS blocks,
    (SELECT COALESCE(SUM(raw_len), 0) FROM body_blocks) AS raw,
    (SELECT COALESCE(SUM(disk_len), 0) FROM body_blocks) AS comp,
    (SELECT COALESCE(SUM(original_bytes), 0) FROM event_body_blobs) AS original,
    (SELECT COALESCE(SUM(stored_bytes), 0) FROM event_body_blobs) AS stored"""


def sample(session_dir: Path, vm_id: str, service_pid: int) -> dict:
    db = session_dir / "session.db"
    bodies = generation_path_for_db(db)
    with closing_ro(db) as conn:
        conn.row_factory = sqlite3.Row
        snapshot = dict(conn.execute(LEDGER_SQL).fetchone())
    return snapshot | {
        "db": _size(db),
        "wal": _size(session_dir / "session.db-wal"),
        "bodies": _size(bodies),
        "process_rss": rss_kb(process_pids(vm_id)),
        "service_rss": rss_kb([str(service_pid)]),
        "writers": writers_of(bodies),
    }


def keep_ledger(session_dir: Path) -> None:
    """Capture the run through the logger's coherent v3 snapshot owner."""
    assert KEEP_DIR
    dest = Path(KEEP_DIR)
    dest.mkdir(parents=True, exist_ok=True)
    root = Path(__file__).resolve().parents[2]
    env = os.environ | {
        "CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER": "/usr/bin/env",
        "RUSTC_WRAPPER": "",
    }
    subprocess.run(
        [
            "cargo",
            "run",
            "--quiet",
            "-p",
            "capsem-logger",
            "--example",
            "snapshot_session_ledger",
            "--",
            str(session_dir),
            str(dest),
        ],
        cwd=root,
        env=env,
        check=True,
    )
    print(f"  kept the ledger in {dest}")


def _disk(snapshot: dict) -> int:
    return snapshot["db"] + snapshot["wal"] + snapshot["bodies"]


def judge(samples: list[dict], failures: list[str]) -> None:
    fail = failures.append
    for prev, cur in itertools.pairwise(samples):
        if cur["reqs"] <= prev["reqs"]:
            fail(f"request count flat ({prev['reqs']} -> {cur['reqs']}): no loop")
    last, kb = samples[-1], 1024
    if last["models"] == 0:
        fail(f"no model_calls rows: {MODEL} was never called through the egress")

    overall = _disk(last) / max(last["reqs"], 1)
    print(f"\n  disk / requests, including the empty schema's fixed floor: {overall / kb:.2f} KB")
    print(f"  bodies indexed: {last['stored'] // kb} KB of {last['original'] // kb} KB seen")
    if last["bodies"]:
        print(f"  blocks: {last['raw'] // kb} KB raw -> {last['comp'] // kb} KB compressed")
        ratio = last["stored"] / last["bodies"]
        print(f"  session.bodies {last['bodies'] // kb} KB: {ratio:.2f}x vs indexed bytes")

    # Every budget is a slope: what one more request costs a long session,
    # from the 10-minute mark (past boot and the schema's fixed floor) to the end.
    if len(samples) < 3:
        fail(f"{len(samples)} samples; the slopes need >= 3 (CAPSEM_ECON_MINUTES >= 15)")
    else:
        a, b = samples[1], samples[-1]
        dreq = max(b["reqs"] - a["reqs"], 1)
        disk = (_disk(b) - _disk(a)) / dreq
        proc = (b["process_rss"] - a["process_rss"]) * kb / dreq
        svc = (b["service_rss"] - a["service_rss"]) * kb / dreq
        print(
            f"  per request, t+10m..end over {dreq} requests: disk {disk / kb:.2f} KB, "
            f"process RSS {proc:.0f} B, service RSS {svc:.0f} B"
        )
        if disk > 6 * kb:
            fail(f"each request adds {disk / kb:.1f} KB on disk (> 6 KB)")
        if proc > 2 * kb:
            fail(f"capsem-process grows {proc / kb:.1f} KB per request")
        if svc > 512:
            fail(f"capsem-service grows {svc:.0f} B per request (should be flat)")

    extra = sorted({w for s in samples for w in s["writers"]} - {"capsem-process"})
    if extra:
        seen = sum(1 for s in samples if set(s["writers"]) - {"capsem-process"})
        fail(
            f"session.bodies open for write by {extra} in {seen}/{len(samples)} samples; "
            "only capsem-process may"
        )


def main() -> int:
    if {"-h", "--help"} & set(sys.argv[1:]):
        print(__doc__)
        return 0
    print(f"\n== preflight: host Ollama serves {MODEL} ==")
    if not host_has_model():
        return 1
    if not host_has_redis_fixture():
        return 1
    service = ServiceInstance()
    service.start()
    assert service.proc is not None, "start() returned without a service process"
    service_pid, client = service.proc.pid, service.client()
    booted, failures, samples = [], [], []
    session_dir = None
    try:
        with registry(service.tmp_dir) as (reference, certificate, _requests):
            print(f"\n== boot a sandboxed VM (profile {CODE_PROFILE_ID}) ==")
            box = boot(service, service.tmp_dir, reference, certificate, "econ")
            booted.append(box["id"])
            session_dir = vm_session_dir(service.tmp_dir, client, box["id"])
            print(f"  box {box['id']}  ledger {session_dir}")

            print(f"\n== {MINUTES} min loop: fetch page -> {MODEL} summary, sampled every 5 min ==")
            seconds = MINUTES * 60
            loop = LOOP.format(sites=SITES, seconds=seconds, model=MODEL)
            result: dict = {}
            runner = threading.Thread(
                target=lambda: result.update(
                    guest(service, box["id"], loop, timeout=seconds + 300)
                ),
                daemon=True,
            )
            runner.start()
            for _ in range(max(1, seconds // SAMPLE_EVERY_S)):
                time.sleep(SAMPLE_EVERY_S)
                s = sample(session_dir, box["id"], service_pid)
                samples.append(s)
                print(
                    f"  t+{len(samples) * 5:>3}m reqs={s['reqs']:5d} models={s['models']:4d} "
                    f"blocks={s['blocks']:4d} db={s['db'] // 1024:6d}K wal={s['wal'] // 1024:6d}K "
                    f"bodies={s['bodies'] // 1024:6d}K process={s['process_rss'] // 1024:5d}M "
                    f"service={s['service_rss'] // 1024:5d}M writers={s['writers']}",
                    flush=True,
                )
            runner.join(timeout=600)

            out = str(result.get("stdout", ""))
            done = next((line for line in out.splitlines() if line.startswith("LOOP DONE")), "")
            print(
                f"\n  guest loop: exit={result.get('exit_code')} {done or '(no completion marker)'}"
            )
            print("  per-sample requests:", [s["reqs"] for s in samples])
            if not done:
                failures.append(
                    f"guest loop printed no completion marker (stdout {out.strip()[:300]!r})"
                )
            judge(samples, failures)
    finally:
        # Before the VM is deleted: the ledger lives under the temp home.
        if KEEP_DIR and session_dir is not None:
            keep_ledger(session_dir)
        for vm_id in booted:
            with contextlib.suppress(Exception):
                client.delete(f"/vms/{vm_id}/delete")
        service.stop()

    print("\nRESULT:", "PASS" if not failures else "FAIL")
    for f in failures:
        print("  -", f)
    return 0 if not failures else 1


if __name__ == "__main__":
    raise SystemExit(main())
