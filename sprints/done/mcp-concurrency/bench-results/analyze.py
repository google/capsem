#!/usr/bin/env python3
"""Median rps + p99 across the 3 runs we captured per phase.

Usage: analyze.py <tag>      e.g. analyze.py pre  or  analyze.py post
       analyze.py compare    print pre vs post side-by-side
"""

import json
import statistics
import sys
from pathlib import Path

ROOT = Path(__file__).parent


def load(tag):
    rows = {"mcp": [], "mitm": []}
    for kind in ("mcp", "mitm"):
        for i in (1, 2, 3):
            p = ROOT / f"bench-{tag}-{kind}-{i}.json"
            if not p.exists():
                continue
            data = json.loads(p.read_text())
            key = "mcp_load" if kind == "mcp" else "mitm_load"
            for level in data[key]["concurrency_levels"]:
                rows[kind].append(level)
    return rows


def medianize(levels):
    """Group by concurrency, compute median rps + p99 across runs."""
    by_c = {}
    for r in levels:
        by_c.setdefault(r["concurrency"], []).append(r)
    out = []
    for c in sorted(by_c):
        runs = by_c[c]
        out.append(
            {
                "concurrency": c,
                "rps": statistics.median(r["rps"] for r in runs),
                "p50": statistics.median(r["p50_ms"] for r in runs),
                "p95": statistics.median(r["p95_ms"] for r in runs),
                "p99": statistics.median(r["p99_ms"] for r in runs),
                "p999": statistics.median(r.get("p999_ms", 0) for r in runs),
                "n": len(runs),
            }
        )
    return out


def fmt(meds):
    print(f"  {'c':>4} {'rps':>8} {'p50':>7} {'p95':>7} {'p99':>7} {'p999':>8} {'n':>3}")
    for r in meds:
        print(
            f"  {r['concurrency']:>4} {r['rps']:>8.1f} {r['p50']:>7.2f} "
            f"{r['p95']:>7.2f} {r['p99']:>7.2f} {r['p999']:>8.2f} {r['n']:>3}"
        )


def show(tag):
    rows = load(tag)
    print(f"=== {tag.upper()} mcp-load (median over n runs) ===")
    fmt(medianize(rows["mcp"]))
    print(f"=== {tag.upper()} mitm-load (median over n runs) ===")
    fmt(medianize(rows["mitm"]))


def compare():
    pre, post = load("pre"), load("post")
    for kind in ("mcp", "mitm"):
        pre_m = {r["concurrency"]: r for r in medianize(pre[kind])}
        post_m = {r["concurrency"]: r for r in medianize(post[kind])}
        common = sorted(set(pre_m) & set(post_m))
        if not common:
            continue
        print(f"=== {kind}-load: pre vs post (median) ===")
        print(
            f"  {'c':>4} {'rps_pre':>8} {'rps_post':>8} {'rps_d%':>8}   "
            f"{'p99_pre':>8} {'p99_post':>8} {'p99_d_ms':>8}"
        )
        for c in common:
            a, b = pre_m[c], post_m[c]
            rps_d = (b["rps"] - a["rps"]) / a["rps"] * 100
            p99_d = b["p99"] - a["p99"]
            print(
                f"  {c:>4} {a['rps']:>8.1f} {b['rps']:>8.1f} {rps_d:>+7.1f}%   "
                f"{a['p99']:>8.2f} {b['p99']:>8.2f} {p99_d:>+8.2f}"
            )


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "pre"
    if cmd == "compare":
        compare()
    else:
        show(cmd)
