"""What a failed asset boot leaves behind, copied out before its scratch goes.

Separate from `assets`, which builds them. This runs on the failure path only,
and it runs *before* the run directory is removed -- which is the whole reason
it is a distinct step in the caller's `except` rather than tidy-up in a
`finally`: release destroys the evidence.

Host-side diagnostics only. `guest/` and `auto_snapshots/` duplicate the
guest's own workspace once per generation, and the same filter keeps the VM
disk image and `session.db` out of `target/`.
"""

from __future__ import annotations

from pathlib import Path

from .config import GateConfig
from .filesystem import copy_file, discard, make_dir
from .proc import Runner


def preserve(runner: Runner, config: GateConfig, *, destination: Path, run_dir: Path) -> None:
    settings = config.assets
    discard(destination)
    make_dir(destination)

    for source in run_dir.rglob("*"):
        relative = source.relative_to(run_dir)
        if set(relative.parts) & set(settings.evidence_prune_dirs):
            continue
        if not source.is_file() or source.suffix not in settings.evidence_suffixes:
            continue
        target = destination / relative
        make_dir(target.parent)
        copy_file(source, target)

    runner.note(f"Preserved asset-gate failure evidence in {destination}")
