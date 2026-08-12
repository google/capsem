#!/usr/bin/env python3
"""ShellCheck every surface that carries shell, and fail closed.

Three surfaces, not one: tracked `*.sh`, every workflow `run:` body, and every
Dockerfile `RUN` body. A linter on one of three is not a linter, it is a
sampling, and the two unlinted ones are where the release logic lives.

Fail closed means a surface yielding no sources is an error, never a pass.
"Found nothing so it was skipped" is how a gate stops being one.

GitHub expressions are masked to a shell variable rather than a literal word:
the runner substitutes each as one value, and masking to a constant makes
`[ "$X" = "y" ]` a constant comparison that ShellCheck rightly flags as SC2050.
That artifact was mistaken for three real release bugs once already.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

from capsem.gate import shellsurfaces

PREAMBLE = "#!/usr/bin/env bash\nset -euo pipefail\n"


def tracked(root: Path, pattern: str) -> list[Path]:
    listed = subprocess.run(
        ["git", "ls-files", "-z", "--", pattern],
        cwd=root,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    return [root / raw.decode() for raw in listed if raw]


def surfaces(root: Path) -> list[tuple[str, dict[str, str]]]:
    """The two extracted surfaces, from the one module that knows how."""
    return [
        ("workflow run: steps", shellsurfaces.workflow_bodies(root / ".github" / "workflows")),
        (
            "Dockerfile RUN bodies",
            shellsurfaces.dockerfile_bodies(
                root / "docker",
                root / "config" / "docker",
                lambda templates: shellsurfaces.rendered_templates(
                    templates, root / "config" / "docker" / "image"
                ),
            ),
        ),
    ]


def shellcheck(paths: list[Path], *, severity: str, exclude: str) -> int:
    if not paths:
        return 0
    argv = ["uv", "run", "shellcheck", f"--severity={severity}", "--shell=bash"]
    if exclude:
        argv += ["--exclude", exclude]
    return subprocess.run([*argv, "--", *[str(p) for p in paths]], check=False).returncode


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("severity")
    parser.add_argument("exclude", nargs="?", default="")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    arguments = parser.parse_args()

    root = arguments.root.resolve()
    failures = 0

    scripts = tracked(root, "*.sh")
    if not scripts:
        print("no tracked shell scripts found; refusing to pass vacuously", file=sys.stderr)
        return 1
    print(f"shellcheck: {len(scripts)} tracked shell scripts")
    failures |= shellcheck(scripts, severity=arguments.severity, exclude=arguments.exclude)

    with tempfile.TemporaryDirectory(prefix="capsem-shellcheck-") as scratch:
        staging = Path(scratch)
        for label, bodies in surfaces(root):
            if not bodies:
                print(f"no {label} found; refusing to pass vacuously", file=sys.stderr)
                return 1
            written = []
            for name, body in bodies.items():
                safe = re.sub(r"[^A-Za-z0-9_.-]", "_", name)
                target = staging / f"{safe}.sh"
                target.write_text(PREAMBLE + body, encoding="utf-8")
                written.append(target)
            print(f"shellcheck: {len(written)} {label}")
            failures |= shellcheck(written, severity=arguments.severity, exclude=arguments.exclude)

    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
