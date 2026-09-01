"""Start the gate on source-keyed Python bytecode.

CPython validates a `.pyc` against the source's *mtime and size*. Two edits of
the same length within one timestamp tick therefore leave bytecode that still
looks valid, and the interpreter runs the old one. During a review of this
package that produced 74 identical false failures from a reference the source
no longer contained; an isolated cache made them vanish without a source
change.

That is not merely bad local feedback. `just test` and both release
commands begin with `uv run --project build_system --frozen capsem-gate`. The local diagnostic records its
commit and source digest, while each release freezes and dispatches that exact
source to a hosted qualifying lane. A stale module could otherwise construct a
plan that does not correspond to the source being diagnosed or published.

So the entry point is this file rather than `capsem_builder.gate.cli`: it
re-execs with an ABI- and source-keyed `pycache_prefix` before anything from
`capsem_builder.gate` is imported. `PYTHONPYCACHEPREFIX` goes into the
environment as well as onto the command line, so every child -- pytest, the
builders, the scripts -- inherits the same isolation.

Nothing here may import `capsem_builder.gate`. Importing it is precisely what
this file exists to do only after the cache is safe, and
`capsem_builder.gate.__init__` carries real code. Only the standard library,
and only a few lines of it, so that this module's own bytecode is something
that never changes.
"""

from __future__ import annotations

import hashlib
import os
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import NoReturn

#: Set to the exact source-keyed generation. Equality with the current tree,
#: environment, and interpreter is how the re-exec knows not to happen twice.
MARKER = "CAPSEM_GATE_PYCACHE"

#: The variable CPython itself reads. Exported rather than only passed as `-X`,
#: because the point is that children inherit it.
PYCACHE = "PYTHONPYCACHEPREFIX"

FALLBACK_STAGE = Path("cache/tools/python/pycache")
GATE_POLICY = Path("config/gate.toml")
CACHE_POLICY = Path("config/cache.toml")


def checkout() -> Path:
    """The checkout this launcher was installed from.

    `capsem_builder.gate.project_root` answers the same question and validates
    the answer, which is the better version -- and unreachable from here,
    because importing it is the thing being deferred.
    """
    return Path(__file__).resolve().parents[2]


def _cache_authority(root: Path) -> Path:
    """Keep private-checkout bytecode in the outer repository cache."""
    policy = root / GATE_POLICY
    if not policy.is_file():
        return root
    raw = tomllib.loads(policy.read_text(encoding="utf-8"))
    variable = raw.get("environment", {}).get("source_checkout")
    selected = os.environ.get(variable, "") if isinstance(variable, str) else ""
    return Path(selected).resolve() if selected else root


def _stage(root: Path) -> Path:
    authority = _cache_authority(root)
    policy = root / CACHE_POLICY
    if not policy.is_file():
        return authority / FALLBACK_STAGE
    raw = tomllib.loads(policy.read_text(encoding="utf-8"))
    return authority / raw["root"] / raw["stages"]["python-pycache"]["path"]


def _python_sources(root: Path) -> tuple[Path, ...]:
    listed = subprocess.run(
        ("git", "-C", str(root), "ls-files", "-co", "--exclude-standard", "-z", "--", "*.py"),
        check=False,
        capture_output=True,
    )
    if listed.returncode == 0:
        candidates = (root / raw.decode() for raw in listed.stdout.split(b"\0") if raw)
        return tuple(path for path in candidates if path.is_file())
    return tuple(sorted(root.rglob("*.py")))


def _source_key(root: Path) -> str:
    digest = hashlib.sha256()
    for path in _python_sources(root):
        digest.update(path.relative_to(root).as_posix().encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def isolated_environment(root: Path | None = None) -> dict[str, str]:
    """A fresh cache prefix, as the environment that selects it.

    Returned rather than applied, so a test can prove the mechanism against a
    module it controls instead of against the gate's own seventy.
    """
    source = (root or checkout()).resolve()
    abi = sys.implementation.cache_tag or "python"
    generation = _stage(source) / f"{abi}-{_source_key(source)}"
    generation.mkdir(parents=True, exist_ok=True)
    return {MARKER: str(generation), PYCACHE: str(generation)}


def main() -> int:
    """Re-exec unless this interpreter matches the current source generation."""
    environment = isolated_environment()
    isolated = (
        os.environ.get(MARKER) == environment[MARKER]
        and os.environ.get(PYCACHE) == environment[PYCACHE]
        and sys.pycache_prefix == environment[PYCACHE]
    )
    if isolated:
        from .gate.cli import main as gate

        return gate()

    return _reexec(environment)


def _reexec(environment: dict[str, str] | None = None) -> NoReturn:
    """Become the same command on an interpreter with a private cache.

    `execv`, not a subprocess: a wrapper process would sit between the terminal
    and the gate for the whole run, taking the signals the gate has to handle
    itself.
    """
    os.environ.update(environment or isolated_environment())
    os.execv(sys.executable, [sys.executable, "-m", "capsem_builder.gate", *sys.argv[1:]])
    raise AssertionError("execv returned")  # pragma: no cover - execv does not
