"""Start the gate on source-keyed Python bytecode.

CPython can accept stale bytecode after same-sized edits in one timestamp tick.

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
import importlib
import os
import shlex
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import BinaryIO, NoReturn

#: Exact source-keyed generation used to prevent a second re-exec.
MARKER = "CAPSEM_GATE_PYCACHE"

#: Exported so child interpreters inherit the same generation.
PYCACHE = "PYTHONPYCACHEPREFIX"
PYTEST_ADDOPTS = "PYTEST_ADDOPTS"
TMPDIR = "TMPDIR"
UV_CACHE = "UV_CACHE_DIR"
RUFF_CACHE = "RUFF_CACHE_DIR"
PNPM_STORE = "npm_config_store_dir"
CARGO_TARGET = "CARGO_TARGET_DIR"
RUSTC_WRAPPER = "RUSTC_WRAPPER"
SCCACHE_DIR = "SCCACHE_DIR"
SCCACHE_CACHE_SIZE = "SCCACHE_CACHE_SIZE"
SCCACHE_BASEDIRS = "SCCACHE_BASEDIRS"
SCCACHE_CLIENT_SIDE = "SCCACHE_CLIENT_SIDE"
SCCACHE_IDLE_TIMEOUT = "SCCACHE_IDLE_TIMEOUT"
SCCACHE_SERVER_UDS = "SCCACHE_SERVER_UDS"

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


def _git_common_checkout(root: Path) -> Path:
    """Resolve linked worktrees before importing the typed cache package."""
    marker = root / ".git"
    if marker.is_dir() or not marker.is_file():
        return root
    try:
        label, raw_git_dir = marker.read_text(encoding="utf-8").strip().split(":", 1)
        if label != "gitdir":
            return root
        git_dir = Path(raw_git_dir.strip()).expanduser()
        if not git_dir.is_absolute():
            git_dir = root / git_dir
        common_file = git_dir / "commondir"
        if not common_file.is_file():
            return root
        common_git_dir = (git_dir / common_file.read_text(encoding="utf-8").strip()).resolve()
    except (OSError, ValueError):
        return root
    return common_git_dir.parent if common_git_dir.name == ".git" else root


def _cache_authority(root: Path) -> Path:
    """Keep private-checkout bytecode in the outer repository cache."""
    policy = root / CACHE_POLICY
    if not policy.is_file():
        return root
    raw = tomllib.loads(policy.read_text(encoding="utf-8"))
    variable = raw.get("authority_environment")
    selected = os.environ.get(variable, "") if isinstance(variable, str) else ""
    return Path(selected).resolve() if selected else _git_common_checkout(root)


def _stage(root: Path, authority: Path | None = None) -> Path:
    storage = _cache_authority(root) if authority is None else authority.resolve()
    policy = root / CACHE_POLICY
    if not policy.is_file():
        return storage / FALLBACK_STAGE
    raw = tomllib.loads(policy.read_text(encoding="utf-8"))
    return storage / raw["root"] / raw["stages"]["python-pycache"]["path"]


def _policy(root: Path) -> dict:
    return tomllib.loads((root / CACHE_POLICY).read_text(encoding="utf-8"))


def _gate_policy(root: Path) -> dict:
    return tomllib.loads((root / GATE_POLICY).read_text(encoding="utf-8"))


def _policy_stage(root: Path, authority: Path, stage_id: str) -> Path:
    raw = _policy(root)
    stage = raw["stages"][stage_id]
    path = Path(stage["path"])
    if stage.get("external", False):
        namespace = hashlib.sha256(str(authority.absolute()).encode()).hexdigest()[:8]
        return path / namespace
    return authority / raw["root"] / path


def _pytest_addopts(cache: Path, basetemp: Path) -> str:
    tokens = shlex.split(os.environ.get(PYTEST_ADDOPTS, ""))
    kept: list[str] = []
    index = 0
    while index < len(tokens):
        token = tokens[index]
        if (
            token in {"-o", "--override-ini"}
            and index + 1 < len(tokens)
            and tokens[index + 1].startswith("cache_dir=")
        ):
            index += 2
            continue
        if token.startswith(("-o=cache_dir=", "--override-ini=cache_dir=")):
            index += 1
            continue
        if token == "--basetemp" and index + 1 < len(tokens):
            index += 2
            continue
        if token.startswith("--basetemp="):
            index += 1
            continue
        kept.append(token)
        index += 1
    return shlex.join((*kept, "-o", f"cache_dir={cache}", f"--basetemp={basetemp}"))


def _test_tmp(root: Path, authority: Path) -> Path:
    return _policy_stage(root, authority, "test-temp") / f"run-{os.getpid()}"


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


def isolated_environment(
    root: Path | None = None, *, authority: Path | None = None
) -> dict[str, str]:
    """A fresh cache prefix, as the environment that selects it.

    Returned rather than applied, so a test can prove the mechanism against a
    module it controls instead of against the gate's own seventy.
    """
    source = (root or checkout()).resolve()
    abi = sys.implementation.cache_tag or "python"
    generation = _stage(source, authority) / f"{abi}-{_source_key(source)}"
    generation.mkdir(parents=True, exist_ok=True)
    return {MARKER: str(generation), PYCACHE: str(generation)}


def contained_environment(root: Path | None = None) -> dict[str, str]:
    """Select every dependency-free tool cache before a bounded child starts."""
    source = (root or checkout()).resolve()
    authority = _cache_authority(source)
    python = isolated_environment(source, authority=authority)
    if not (source / CACHE_POLICY).is_file() or not (source / GATE_POLICY).is_file():
        return python
    generation = Path(python[PYCACHE])
    pytest = _policy_stage(source, authority, "python-pytest") / generation.name
    test_tmp = _test_tmp(source, authority)
    test_tmp.mkdir(parents=True, exist_ok=True)
    gate = _gate_policy(source)
    toolchain = gate["toolchain"]
    uv = _policy_stage(source, authority, "python-uv")
    ruff = _policy_stage(source, authority, "python-ruff")
    rust = _policy_stage(source, authority, "rust-sccache")
    cache = _policy(source)
    environment = {
        **python,
        PYTEST_ADDOPTS: _pytest_addopts(pytest, test_tmp / "pytest"),
        TMPDIR: str(test_tmp),
        UV_CACHE: str(uv),
        RUFF_CACHE: str(ruff),
        PNPM_STORE: str(_policy_stage(source, authority, "node-pnpm")),
        CARGO_TARGET: str(_policy_stage(source, authority, "cargo")),
        SCCACHE_DIR: str(rust),
        SCCACHE_CACHE_SIZE: f"{cache['stages']['rust-sccache']['max_size_bytes'] // 1024**3}G",
        SCCACHE_BASEDIRS: str(source),
        SCCACHE_CLIENT_SIDE: "1" if toolchain["compiler_cache_client_side"] else "0",
        SCCACHE_IDLE_TIMEOUT: str(toolchain["compiler_cache_idle_timeout_seconds"]),
        SCCACHE_SERVER_UDS: str(rust / toolchain["compiler_cache_socket_name"]),
    }
    if shutil.which(toolchain["compiler_cache_command"]) is not None:
        environment[RUSTC_WRAPPER] = toolchain["compiler_cache_command"]
    return environment


def hold_environment(root: Path | None = None) -> None:
    """Lease both exact Python generations for the lifetime of this process."""
    source = (root or checkout()).resolve()
    authority = _cache_authority(source)
    generation = Path(isolated_environment(source, authority=authority)[PYCACHE])
    if not (source / CACHE_POLICY).is_file():
        _hold_generation(generation)
        return
    pytest = _policy_stage(source, authority, "python-pytest") / generation.name
    test_tmp = _test_tmp(source, authority)
    _hold_generation(generation)
    _hold_generation(pytest)
    _hold_generation(test_tmp)


def _hold_generation(generation: Path) -> BinaryIO:
    """Hold a shared lifetime lease that makes routine pruning skip this generation."""
    from .cache.leases import retain_path

    return retain_path(generation.with_name(f".{generation.name}.lock"))


def _launch(implementation: str, reexec_module: str) -> int:
    """Enter one implementation only after every tool cache is contained."""
    environment = contained_environment()
    isolated = (
        os.environ.get(MARKER) == environment[MARKER]
        and os.environ.get(PYCACHE) == environment[PYCACHE]
        and sys.pycache_prefix == environment[PYCACHE]
    )
    if isolated:
        os.environ.update(environment)
        hold_environment()
        entrypoint = importlib.import_module(implementation).main
        return entrypoint()

    return _reexec(environment, reexec_module)


def main() -> int:
    """Run the build gate beneath the source-keyed cache authority."""
    return _launch("capsem_builder.gate.cli", "capsem_builder.gate")


def cache_main() -> int:
    """Run cache control without creating bytecode beside its own source."""
    return _launch("capsem_builder.cache.cli", "capsem_builder.cache")


def builder_main() -> int:
    """Run image building without creating bytecode beside its own source."""
    return _launch("capsem_builder.image.cli", "capsem_builder.image")


def _reexec(
    environment: dict[str, str] | None = None, module: str = "capsem_builder.gate"
) -> NoReturn:
    """Become the same command on an interpreter with a private cache.

    `execv`, not a subprocess: a wrapper process would sit between the terminal
    and the gate for the whole run, taking the signals the gate has to handle
    itself.
    """
    os.environ.update(environment or isolated_environment())
    os.execv(sys.executable, [sys.executable, "-m", module, *sys.argv[1:]])
    raise AssertionError("execv returned")  # pragma: no cover - execv does not
