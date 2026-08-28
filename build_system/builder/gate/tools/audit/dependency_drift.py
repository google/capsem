"""Report dependency-graph changes that look like a supply-chain attack.

Never blocking. A supply-chain signal that fails the build is a signal people
learn to route around, and the structural defence is already elsewhere: cargo
compiles inside a loopback-only network namespace, so a build script reaching a
command-and-control host gets no route. This is the part that was missing --
saying so *before* the build, and saying which package.

The shape it looks for is the shape that actually happened. On 2026-08-20 the
`arrayref` account was compromised: every version yanked, one malicious release
published as the only installable one. `arrayref` is roughly two hundred lines
of `macro_rules!` and had no dependencies since 2015. The release declared one:
`proc-macro1`, a typosquat of `proc-macro2`, whose build script fetched and ran
a remote binary at compile time.

A crate gaining its first ever dependency, or gaining a build script, is
therefore the highest-signal thing available here -- far better than scanning
for known-bad names, because the name was new.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any

from capsem_builder.gate import project_root

ROOT = project_root()

#: Regenerate with `--write` when a change is understood and intended.
INVENTORY = ROOT / "config" / "dependency-inventory.json"

#: cargo marks a build script as this target kind.
BUILD_SCRIPT = "custom-build"


def resolved_graph(root: Path) -> dict[str, dict[str, Any]]:
    """Every package cargo resolves, with the two facts worth watching."""
    result = subprocess.run(
        ("cargo", "metadata", "--locked", "--format-version", "1"),
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    graph: dict[str, dict[str, Any]] = {}
    for package in json.loads(result.stdout)["packages"]:
        name = package["name"]
        builds = any(
            BUILD_SCRIPT in target.get("kind", ()) for target in package.get("targets", ())
        )
        # Declared rather than resolved: what the crate *asked* for is the claim
        # that changed, and a feature-gated addition is exactly as interesting.
        deps = sorted({dep["name"] for dep in package.get("dependencies", ())})
        previous = graph.get(name)
        if previous is not None:
            # Two versions of one crate in the graph: watch the union, so a
            # second copy cannot hide a new dependency behind the first.
            builds = builds or previous["build_script"]
            deps = sorted(set(deps) | set(previous["dependencies"]))
        graph[name] = {"build_script": builds, "dependencies": deps}
    return graph


def drift(current: dict[str, dict[str, Any]], known: dict[str, dict[str, Any]]) -> list[str]:
    """What changed, worst first."""
    findings: list[str] = []
    for name, facts in sorted(current.items()):
        before = known.get(name)
        if before is None:
            note = f"NEW package {name}"
            if facts["build_script"]:
                note += " -- and it runs a build script at compile time"
            findings.append(note)
            continue
        if facts["build_script"] and not before.get("build_script"):
            findings.append(f"{name} GAINED A BUILD SCRIPT, which runs at compile time")
        gained = sorted(set(facts["dependencies"]) - set(before.get("dependencies", ())))
        if not gained:
            continue
        if not before.get("dependencies"):
            findings.append(
                f"{name} had NO dependencies and now declares {', '.join(gained)} "
                "-- this is the shape the arrayref compromise took"
            )
        else:
            findings.append(f"{name} declares new dependencies: {', '.join(gained)}")
    for name in sorted(set(known) - set(current)):
        findings.append(f"{name} is gone from the graph")
    return findings


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument(
        "--write",
        action="store_true",
        help="record the current graph as the accepted baseline",
    )
    args = parser.parse_args(argv)

    current = resolved_graph(args.root)
    if args.write:
        INVENTORY.write_text(json.dumps(current, indent=1, sort_keys=True) + "\n")
        print(f"recorded {len(current)} packages in {INVENTORY.relative_to(ROOT)}")
        return 0

    if not INVENTORY.is_file():
        print(f"no dependency inventory at {INVENTORY}; run with --write to seed it")
        return 0

    known = json.loads(INVENTORY.read_text())
    findings = drift(current, known)
    if not findings:
        print(f"dependency graph unchanged across {len(current)} packages")
        return 0

    print(f"dependency drift across {len(current)} packages -- {len(findings)} change(s):")
    for finding in findings:
        print(f"  {finding}")
    print(
        "\nThis is a report, not a refusal. Read the change before building: "
        "`cargo update` and `cargo tree` never run a build script, and `cargo "
        "build` does. Accept it with `--write` once understood."
    )
    return 0
