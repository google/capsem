"""The isolated home a gate run works in, and giving it back cleanly.

Three recipes hand-wrote this: `_test-candidate-run`, `smoke`, and the asset
gate's run directory. Each set up an isolated `CAPSEM_HOME`, exported the same
four variables, installed an EXIT trap to stop whatever service ended up in it,
and got slightly different details right.

The order is the whole thing, and it is not obvious in any of them:

  the machine lock is taken *before* the home is wiped, which is why the
  lockfile lives outside the home -- a lock inside the tree about to be
  removed is a lock on an inode nobody else can see

  the service is stopped *before* the run directory is deleted, because
  stopping it is what flushes `serial.log`, and that file is what a boot
  failure is argued from

  evidence is copied out before either, because both destroy it

As a `Resource` these stop being lines in a `finally` and become the shape of a
stack. The lock is not held here: `GateCommand` takes it first, so it outlives
this and is released last.
"""

from __future__ import annotations

import hashlib
import os
import shutil
from pathlib import Path

from . import pidfiles
from .config import GateConfig
from .fileactions import digest_of
from .lifecycle import Resource


class Workspace(Resource, name="workspace"):
    """An isolated `CAPSEM_HOME`, wiped on the way in and cleared on the way out.

    Wiped on entry rather than on exit: a run that crashed leaves its home for
    inspection, and the next run is the one that no longer needs it.
    """

    def __init__(self, config: GateConfig) -> None:
        self._config = config
        self._settings = config.workspace
        self.home = config.path(self._settings.home)
        root_id = hashlib.blake2s(os.fsencode(config.root.resolve()), digest_size=4).hexdigest()
        self.run_dir = Path(self._settings.run_dir.format(root_id=root_id))
        self._preserved: Path | None = None

    def _remove_run_dir(self) -> None:
        """Remove only this config-derived, user-owned temporary run tree."""
        if self.run_dir.is_symlink():
            raise RuntimeError(f"workspace run directory {self.run_dir} must not be a symlink")
        if not self.run_dir.exists():
            return
        if self.run_dir.stat().st_uid != os.getuid():
            raise RuntimeError(f"workspace run directory {self.run_dir} has the wrong owner")
        shutil.rmtree(self.run_dir)

    def environment(self) -> dict[str, str]:
        """What every command inside this workspace inherits.

        Exported once here rather than by each invocation remembering, which is
        how one of them stopped remembering and wrote into the developer's own
        `~/.capsem`.

        A method, not a property: `Resource.environment` is what
        `GateCommand.execute` calls on everything it acquired, and a property
        here made that call `TypeError: 'dict' object is not callable` against
        the one resource every isolated command holds.
        """
        names = self._config.environment
        return {
            names.home: str(self.home),
            names.run_dir: str(self.run_dir),
            names.benchmark_root: str(self._config.path(self._settings.benchmark_root)),
            names.coverage_file: str(self._config.path(self._settings.coverage_file)),
        }

    # -- Resource ----------------------------------------------------------

    def acquire(self) -> None:
        from .disk import _remove_tree

        if self.home.is_dir():
            _remove_tree(self.home, self._config.root)
        self._remove_run_dir()
        for relative in self._settings.seeded_dirs:
            (self.home / relative).mkdir(parents=True, exist_ok=True)
        self.run_dir.mkdir(mode=0o700, parents=True)
        self._config.path(self._settings.coverage_file).parent.mkdir(parents=True, exist_ok=True)
        # Deliberately not the benchmark root: `just test-clean` runs several modules
        # through one workspace, the VM recordings come from the functional
        # one, and a later module clearing them is why a fortnight of gates
        # left that directory empty.

    def preserve(self, error: BaseException) -> None:
        """Copy the host-side diagnostics out before anything removes them."""
        from .disk import _remove_tree

        destination = self._config.path(self._settings.evidence_dir)
        if destination.is_dir():
            _remove_tree(destination, self._config.root)
        destination.mkdir(parents=True, exist_ok=True)

        # The service has to stop first: it SIGTERMs every VM process, and that
        # is what flushes process.log and serial.log into the run directory.
        pidfiles.stop_gate_service(self.run_dir, self._config.pidfiles)
        self._copy_logs(destination)
        self._preserved = destination

    def release(self) -> None:
        pidfiles.stop_gate_service(self.run_dir, self._config.pidfiles)
        self._remove_run_dir()

    # -- evidence ----------------------------------------------------------

    def _copy_logs(self, destination: Path) -> None:
        assets = self._config.assets
        for source in sorted(self.run_dir.rglob("*")):
            relative = source.relative_to(self.run_dir)
            if set(relative.parts) & set(assets.evidence_prune_dirs):
                continue
            if not source.is_file() or source.suffix not in assets.evidence_suffixes:
                continue
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read_bytes())

    @property
    def preserved(self) -> Path | None:
        """Where the evidence went, if a failure put any there."""
        return self._preserved


def digest(path: Path, config: GateConfig) -> str:
    """A file's digest in whatever algorithm the run log records."""
    return digest_of(path, algorithm=config.runlog.artifact_digest)
