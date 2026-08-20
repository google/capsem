#!/usr/bin/env python3
"""Replay a release qualification lane on this machine.

`just test` runs the lanes' *steps* -- the rehearsal composes the same
fragments a release does -- but not their *layout*. A release qualifies from a
prefix carrying only tracked files, where `target/debug` and `target/config`
hold staged input rather than build output, and that difference is where the
0.6.0 binaries lost four dispatches: a hardcoded binary path, a missing profile
tree, a missing tool, and a nested pytest inheriting an environment that no
longer said which lane it was in. Each cost forty minutes to see and minutes to
fix.

This runs the lane here instead. It fabricates a cohort out of what the machine
already built, exports the environment the workflow exports, and hands both to
the same `qualify-*` command CI runs. The last of those four defects had
survived four dispatches reporting only `subprocess exited 1`; replayed here it
was diagnosed in one pass.

Not a plan action, so it may invoke the gate: `test_gate_no_nested_commands`
scopes that rule to `src/capsem/gate`, where a nested command would deadlock on
a lock its own parent holds. This is an operator tool, run deliberately.

Needs a tree that has already built its assets and packages -- the same
precondition `capsem-gate test-rehearsal` states, and for the same reason.

`just replay-release-lane [binaries|assets]` covers the common case. The rarer
flags -- resuming at a step, or running the installed-package proof -- are
typed at this script directly, because a recipe may only hand a variadic over
as one quoted argument and splitting it back is how injection gets in.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

#: The installed-package proof, which is where a local replay must stop. It
#: purges the host `capsem`, deletes `~/.capsem` and reinstalls from the
#: channel under test. That is correct on a runner that is deleted minutes
#: later and destructive on a developer machine -- `module_rehearsal` refuses
#: it for the same reason, and this tool learned that the expensive way.
DESTRUCTIVE_STEP = "glowup.package"

#: Where the fabricated cohort lands. Distinct from the rehearsal's own paths so
#: a replay never consumes or clobbers what a running `just test` staged.
WORK = "target/replay-lane"


def _config() -> dict:
    return tomllib.loads((ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))


def _run(argv: list[str], *, env: dict[str, str] | None = None) -> int:
    print(f"+ {' '.join(argv)}", flush=True)
    return subprocess.run(argv, cwd=ROOT, env=env, check=False).returncode


def fabricate(channel: str, profile: str, version: str) -> dict:
    """Build a digest-verified cohort from what this machine already has."""
    package = f"{WORK}-package/Capsem_{version}_amd64.deb"
    result = subprocess.run(
        [
            "uv", "run", "python",
            str(ROOT / "scripts" / "rehearse-release-cohort.py"),
            "--assets-dir", "assets",
            "--bin-dir", "target/debug",
            "--packages-dir", "dist",
            "--work-dir", WORK,
            "--inputs-dir", f"{WORK}-inputs",
            "--package", package,
            "--content-root", f"{WORK}-content",
            "--before-inputs", f"{WORK}-before",
            "--channel", channel,
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout[result.stdout.index("{") :])


def released_environment(cohort: dict, channel: str) -> dict[str, str]:
    """The variables the workflow exports before it calls the lane.

    Kept beside the workflow that sets them, because a replay that exports a
    different set proves a lane nobody runs.
    """
    workspace = str(ROOT)
    return {
        **os.environ,
        "CAPSEM_RELEASE_PACKAGE": cohort["package"],
        "CAPSEM_RELEASE_BIN_DIR": f"{workspace}/target/debug",
        "CAPSEM_TEST_BINARY": f"{workspace}/target/debug/capsem",
        "CAPSEM_RELEASE_INPUT_DIR": cohort["inputs"],
        "CAPSEM_RELEASE_CHANNEL": channel,
        "CAPSEM_RELEASE_TRANSITION": "auto",
        "CAPSEM_RELEASE_BEFORE_MANIFEST": cohort["before_manifest"],
        "CAPSEM_RELEASE_AFTER_MANIFEST": cohort["manifest"],
        "CAPSEM_RELEASE_BEFORE_PROFILE_INPUTS": cohort["before_profile_inputs"],
        "CAPSEM_RELEASE_AFTER_PROFILE_INPUTS": cohort["inputs"],
    }


def main(argv: list[str] | None = None) -> int:
    config = _config()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lane", choices=("binaries", "assets"), default="binaries")
    parser.add_argument("--channel", default="stable")
    parser.add_argument("--profile", default=config["suites"]["pytest"]["base_profile"])
    parser.add_argument(
        "--activation-ready",
        default="true",
        help="assets lane only: whether the profile may activate",
    )
    parser.add_argument("--from", dest="resume", help="resume the lane at this step")
    parser.add_argument(
        "--until",
        default=DESTRUCTIVE_STEP,
        help="stop before this step; the default is the installed-package proof",
    )
    parser.add_argument(
        "--install-on-this-machine",
        action="store_true",
        help=(
            "run the installed-package proof too. It purges the host capsem, "
            "deletes ~/.capsem and reinstalls from the channel under test -- "
            "correct on a disposable runner, and not on a machine you use."
        ),
    )
    args = parser.parse_args(argv)

    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"][
        "package"
    ]["version"]

    print(f"fabricating a {args.channel} cohort for {version} ...", flush=True)
    cohort = fabricate(args.channel, args.profile, version)
    env = released_environment(cohort, args.channel)

    if args.lane == "binaries":
        command = ["just", "qualify-binaries", str(ROOT)]
    else:
        command = [
            "just", "qualify-assets", cohort["inputs"], args.profile,
            str(ROOT), args.activation_ready,
        ]
    if args.resume:
        command += ["--from", args.resume]
    if args.install_on_this_machine:
        print(
            "running the installed-package proof: this purges the host capsem "
            "and deletes ~/.capsem",
            flush=True,
        )
    else:
        command += ["--until", args.until]
    return _run(command, env=env)


if __name__ == "__main__":
    sys.exit(main())
