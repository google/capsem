"""Shared pytest fixtures for inspect-capsem tests."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from collections.abc import Iterator
from pathlib import Path

import inspect_capsem._lifecycle as lifecycle_mod
import pytest
from inspect_capsem import CapsemSandboxEnvironment


def _sanitize_basetemp_namespace(namespace: str) -> str:
    """Return a filesystem-safe basetemp subdirectory namespace."""
    cleaned = "".join(ch if ch.isalnum() or ch in ("-", "_") else "-" for ch in namespace)
    cleaned = cleaned.strip("-_")
    return cleaned or "inspect-capsem"


def _namespaced_basetemp(basetemp: str | None, env: dict[str, str] | None = None) -> str | None:
    """Give each named pytest invocation its own directory under `--basetemp`.

    The gate exports one `--basetemp` for every step, and pytest empties it
    when a session starts. Run one after another that went unnoticed; run side
    by side, a second suite deleted the first one's `tmp_path` files mid-test.
    An xdist worker is already handed a directory inside its controller's.
    """
    source = os.environ if env is None else env
    if not basetemp or source.get("PYTEST_XDIST_WORKER"):
        return basetemp
    namespace = source.get("CAPSEM_TEST_RUN_ID", "").strip() or "inspect-capsem"
    return str(Path(basetemp) / _sanitize_basetemp_namespace(namespace))


@pytest.hookimpl(tryfirst=True)
def pytest_configure(config: pytest.Config) -> None:
    namespaced = _namespaced_basetemp(config.option.basetemp)
    if namespaced is not None and namespaced != config.option.basetemp:
        Path(namespaced).parent.mkdir(parents=True, exist_ok=True)
        config.option.basetemp = namespaced
        # `sdkchecks.py` runs `pytest` from the package root with
        # `testpaths = ["tests"]` and no CLI path args (and lints/typechecks
        # only `source` + `tests/`), so pytest loads `tests/conftest.py` during
        # collection after `_pytest.tmpdir.pytest_configure` has already
        # constructed `config._tmp_path_factory` from `config.option.basetemp`.
        tmp_factory = getattr(config, "_tmp_path_factory", None)
        if tmp_factory is not None:
            tmp_factory._given_basetemp = Path(namespaced).resolve()
            tmp_factory._basetemp = None


_PORTABLE_TIMEOUT_SHIM = f"""#!{sys.executable}
import os
import signal
import subprocess
import sys


def _parse_secs(raw: str) -> float:
    s = raw.strip()
    for suffix, mult in (("s", 1.0), ("m", 60.0), ("h", 3600.0), ("d", 86400.0)):
        if s.endswith(suffix):
            return float(s[: -len(suffix)]) * mult
    return float(s)


def _parse_sig(raw: str) -> int:
    s = raw.strip().upper().removeprefix("SIG")
    if s.isdigit():
        return int(s)
    return int(getattr(signal, f"SIG{{s}}", signal.SIGTERM))


def main(argv: list[str]) -> int:
    if "--version" in argv:
        print("timeout (GNU coreutils portable shim) 1.0")
        return 0
    sig = int(signal.SIGTERM)
    kill_after: float | None = None
    i = 0
    while i < len(argv) and argv[i].startswith("-"):
        arg = argv[i]
        if arg == "--":
            i += 1
            break
        if arg in ("-k", "--kill-after"):
            kill_after = _parse_secs(argv[i + 1])
            i += 2
        elif arg.startswith("--kill-after="):
            kill_after = _parse_secs(arg.split("=", 1)[1])
            i += 1
        elif arg.startswith("-k") and len(arg) > 2:
            kill_after = _parse_secs(arg[2:])
            i += 1
        elif arg in ("-s", "--signal"):
            sig = _parse_sig(argv[i + 1])
            i += 2
        elif arg.startswith("--signal="):
            sig = _parse_sig(arg.split("=", 1)[1])
            i += 1
        elif arg.startswith("-s") and len(arg) > 2:
            sig = _parse_sig(arg[2:])
            i += 1
        else:
            i += 1
    if len(argv) - i < 2:
        return 125
    duration = _parse_secs(argv[i])
    cmd = argv[i + 1 :]
    proc = subprocess.Popen(cmd, start_new_session=True)
    try:
        rc = proc.wait(timeout=duration)
        return 128 + (-rc) if rc < 0 else rc
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, sig)
        except ProcessLookupError:
            pass
        if sig != int(signal.SIGKILL):
            try:
                proc.wait(timeout=kill_after if kill_after is not None else 1.0)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                proc.wait()
            return 124
        proc.wait()
        return 137


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
"""


def write_portable_timeout_shim(bin_dir: Path) -> Path:
    """Write a GNU-compatible `timeout` shim script into `bin_dir` and return its path."""
    bin_dir.mkdir(parents=True, exist_ok=True)
    shim = bin_dir / "timeout"
    shim.write_text(_PORTABLE_TIMEOUT_SHIM, encoding="utf-8")
    shim.chmod(0o755)
    return shim


def _host_has_gnu_timeout() -> bool:
    if shutil.which("timeout") is None:
        return False
    try:
        out = subprocess.run(
            ["timeout", "--version"], capture_output=True, text=True, check=False
        ).stdout
    except OSError:
        return False
    return "GNU coreutils" in out and "uutils" not in out


@pytest.fixture(scope="session")
def _portable_timeout_bin_dir(tmp_path_factory: pytest.TempPathFactory) -> Path:
    bin_dir = tmp_path_factory.mktemp("portable-timeout-bin")
    write_portable_timeout_shim(bin_dir)
    return bin_dir


@pytest.fixture(autouse=True)
def _clear_process_owned_vms(
    _portable_timeout_bin_dir: Path, monkeypatch: pytest.MonkeyPatch
) -> Iterator[None]:
    monkeypatch.delenv("CAPSEM_VM_PREFIX", raising=False)
    if not _host_has_gnu_timeout():
        cur_path = os.environ.get("PATH", "/usr/bin:/bin")
        monkeypatch.setenv("PATH", f"{_portable_timeout_bin_dir}:{cur_path}")
    lifecycle_mod._PROCESS_OWNED_VMS.clear()
    CapsemSandboxEnvironment._active_environments.clear()
    yield
    lifecycle_mod._PROCESS_OWNED_VMS.clear()
    CapsemSandboxEnvironment._active_environments.clear()
