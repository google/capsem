"""Generate static, downloadable page data from exact Git snapshots and captured evidence."""

from __future__ import annotations

import csv
import datetime as dt
import json
import tempfile
from collections import Counter
from pathlib import Path

from .artifacts import refresh
from .benchmarks import normalize
from .code import COMPONENTS, extract_snapshot, loc
from .dependencies import declarations, lock_packages
from .github import (
    GitHub,
    authenticated,
    monthly_outcomes,
    publication_timings,
    success_intervals,
    update_attempts,
)
from .history import git, monthly_snapshots
from .test_inventory import counts
from .tools import source_scanner


def statistics(repo: Path, scanner: Path, ref: str = "HEAD") -> dict:
    snapshots = monthly_snapshots(repo, ref)
    cache = {}
    result = []
    for snapshot in snapshots:
        commit = snapshot["commit"]
        if commit not in cache:
            with tempfile.TemporaryDirectory(prefix="capsem-docs-history-") as temp:
                root = Path(temp)
                paths = extract_snapshot(repo, commit, root)
                declared, members, _, _ = declarations(root, paths)
                test_counts = counts(root, paths)
                inventory = loc(root, paths, scanner)
                lines = Counter()
                client_lines = Counter()
                for row in inventory:
                    if not row["generated_excluded"]:
                        lines[row["component"]] += row["code_loc"]
                        if row["component"] == "clients":
                            client_lines[row["subcomponent"]] += row["code_loc"]
                dependencies = {}
                for ecosystem in ["rust", "npm", "python"]:
                    direct = {
                        r["package"]
                        for r in declared
                        if r["ecosystem"] == ecosystem and not r["internal"]
                    }
                    resolved, locks = lock_packages(root, paths, ecosystem)
                    versions = Counter(name for name, _ in resolved)
                    dependencies[ecosystem] = {
                        "direct": len(direct),
                        "resolvedNames": len(versions) if locks else None,
                        "resolvedVersions": len(resolved) if locks else None,
                        "duplicateNames": sum(n > 1 for n in versions.values()) if locks else None,
                        "lockfiles": locks,
                    }
                edges = sorted(
                    {
                        (r["workspace"], r["package"], r["kind"])
                        for r in declared
                        if r["ecosystem"] == "rust" and r["internal"]
                    }
                )
                cache[commit] = {
                    "loc": {key: lines[key] for key in COMPONENTS},
                    "clientLoc": dict(client_lines),
                    "tests": test_counts,
                    "dependencies": dependencies,
                    "crates": sorted(doc["package"]["name"] for doc in members.values()),
                    "edges": [{"from": a, "to": b, "kind": kind} for a, b, kind in edges],
                }
        result.append({**snapshot, **cache[commit]})
        print(f"Monthly snapshot {snapshot['date']}: {commit[:12]}", flush=True)
    return {
        "schema": "capsem.docs.statistics.v1",
        "sourceCommit": git(repo, "rev-parse", ref),
        "generatedAt": dt.datetime.now(dt.UTC).isoformat(),
        "snapshots": result,
        "methodology": {
            "cadence": "UTC calendar month-end, initial commit and latest partial month",
            "source": "Original main lineage through the explicit history-restoration merge, followed by current main.",
            "loc": "scc 4.1.0 nonblank/noncomment source; generated and vendor code excluded.",
            "tests": "Static definitions, not parameterized expansions, platform executions or doctests.",
        },
    }


def generate(
    repo: Path, output: Path, scanner: Path | None = None, *, offline: bool = False
) -> None:
    seed = repo / "web/docs/data"
    output.mkdir(parents=True, exist_ok=True)
    data = statistics(repo, scanner or source_scanner(repo))
    captured = json.loads((seed / "github.json").read_text())
    attempts = captured["attempts"]
    publications = captured["publications"]
    if not offline and authenticated():
        attempts = update_attempts(attempts, "google/capsem", GitHub())
        current = GitHub().pages("repos/google/capsem/releases")
        publications = list({r["tag_name"]: r for r in publications + current}.values())
    data["releaseOutcomes"] = monthly_outcomes(attempts)
    data["releaseAttempts"] = attempts
    data["releaseSuccesses"] = success_intervals(attempts)
    data["releasePublications"] = publication_timings(publications, attempts)
    data["githubObservedAt"] = (
        captured["observedAt"] if offline or not authenticated() else data["generatedAt"]
    )
    benchmark_snapshot = json.loads((seed / "benchmarks.json").read_text())
    if not offline and authenticated():
        benchmark_snapshot = refresh(benchmark_snapshot, GitHub())
    benchmark_data = normalize(benchmark_snapshot)
    for name, value in [
        ("statistics.json", data),
        ("benchmarks.json", benchmark_data),
        ("benchmark-records.json", benchmark_snapshot),
    ]:
        (output / name).write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")
    with (output / "monthly-statistics.csv").open("w", newline="") as stream:
        rows = [
            {
                "date": r["date"],
                "commit": r["commit"],
                "crates": len(r["crates"]),
                **{key + "_loc": value for key, value in r["loc"].items()},
                "test_definitions": sum(r["tests"].values()),
            }
            for r in data["snapshots"]
        ]
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
