"""Validate the session DB produced by a capsem-doctor run.

Boots the VM with capsem-doctor, captures the session ID, then inspects
the session.db and main.db to verify that all telemetry pipelines recorded
data correctly during the diagnostic run.

Capsem-doctor exercises network (allowed + denied domains), filesystem
(test file writes), MCP (tool discovery + invocation), and hermetic
model-shaped traffic through the local mock server. This test validates
that all of those events were captured.

Usage:
    python3 build_system/scripts/doctor/doctor_session_test.py              # uses cache/target/cargo/debug/capsem
    python3 build_system/scripts/doctor/doctor_session_test.py --binary ./capsem --assets ./assets

Ironbank note: this script is a black-box ledger validator. Do not weaken it
into status-only checks, row-exists checks, skipped cases, slow/optional cases,
or Rust-internal expectations. Release-critical cases belong in
tests/ironbank/ and must assert the full public ledger.
"""

import argparse
import functools
import importlib.util
import os
import shlex
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Protocol, cast

from capsem_builder.gate.tools.doctor.doctor_session_verify import (
    verify_session as _verify_session,
)

PROJECT_ROOT = Path(os.environ.get("CAPSEM_REPOSITORY_ROOT", Path.cwd())).resolve()
SCRIPT_DIR = PROJECT_ROOT / "build_system" / "scripts" / "test"


class _MockServerModule(Protocol):
    def start_mock_server(self) -> tuple[subprocess.Popen[str], dict[str, Any]]: ...

    def stop_process(self, process: subprocess.Popen[str] | None) -> None: ...


@functools.cache
def _mock_server_module() -> _MockServerModule:
    helper = SCRIPT_DIR / "mock_server.py"
    sys.path.insert(0, str(SCRIPT_DIR))
    spec = importlib.util.spec_from_file_location("capsem_doctor_mock_server", helper)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load shared mock-server helper: {helper}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return cast(_MockServerModule, module)


def start_mock_server() -> tuple[subprocess.Popen[str], dict[str, Any]]:
    return _mock_server_module().start_mock_server()


def stop_process(process: subprocess.Popen[str] | None) -> None:
    _mock_server_module().stop_process(process)


BOLD = "\033[1m"
DIM = "\033[2m"
GREEN = "\033[32m"
RED = "\033[31m"
YELLOW = "\033[33m"
CYAN = "\033[36m"
RESET = "\033[0m"

MOCK_SERVER_ENV = "CAPSEM_MOCK_SERVER_BASE_URL"
DOCTOR_COMMAND = "capsem-doctor"


def _capsem_home() -> Path:
    env = os.environ.get("CAPSEM_HOME")
    if env:
        return Path(env)
    return Path.home() / ".capsem"


def _run_dir() -> Path:
    env = os.environ.get("CAPSEM_RUN_DIR")
    if env:
        return Path(env)
    return _capsem_home() / "run"


CAPSEM_HOME = _capsem_home()
PERSISTENT_DIR = _run_dir() / "persistent"
SERVICE_SOCKET = _run_dir() / "service.sock"
MAIN_DB = CAPSEM_HOME / "sessions" / "main.db"


def _parse_created_session_id(stdout: str) -> str:
    for line in stdout.splitlines():
        stripped = line.strip()
        if stripped:
            return stripped.split()[0]
    raise RuntimeError("capsem create returned no session id")


def _cli_env(assets_dir: str) -> dict[str, str]:
    return {
        **os.environ,
        "CAPSEM_ASSETS_DIR": assets_dir,
        "RUST_LOG": "capsem=warn",
    }


def run_doctor(binary: str, assets_dir: str, mock_base_url: str) -> tuple[str, Path, int]:
    """Boot the VM with capsem-doctor, return (session_id, exit_code).

    Uses an explicit named session so the post-run session DB remains
    available for ledger validation. `capsem run` intentionally cleans up its
    ephemeral session directory after exit.
    """
    env = _cli_env(assets_dir)

    session_name = f"doctor-ledger-{os.getpid()}-{int(time.time())}"
    print(f"{BOLD}Creating VM for capsem-doctor ...{RESET}")
    create = subprocess.run(
        [
            binary,
            "create",
            "-n",
            session_name,
            "--ram",
            "2",
            "--cpu",
            "2",
            "-e",
            f"{MOCK_SERVER_ENV}={mock_base_url}",
        ],
        env=env,
        capture_output=True,
        text=True,
        timeout=180,
    )
    if create.returncode != 0:
        if create.stdout.strip():
            print(create.stdout.strip())
        if create.stderr.strip():
            print(create.stderr.strip(), file=sys.stderr)
        sys.exit(create.returncode)
    session_id = _parse_created_session_id(create.stdout)
    session_dir = PERSISTENT_DIR / session_id

    if not session_dir.exists():
        print(f"{RED}FAIL: no persistent session directory found in {PERSISTENT_DIR}{RESET}")
        print(f"    {YELLOW}--- stderr ---{RESET}")
        for line in create.stderr.strip().splitlines()[:30]:
            print(f"    {line}")
        sys.exit(1)

    print(f"{BOLD}Booting VM with capsem-doctor ...{RESET}")
    proc = subprocess.run(
        [
            binary,
            "exec",
            session_id,
            f"export {MOCK_SERVER_ENV}={shlex.quote(mock_base_url)}; {DOCTOR_COMMAND}",
            "--timeout",
            "220",
        ],
        env=env,
        capture_output=True,
        text=True,
        timeout=240,
    )
    exit_code = proc.returncode
    if proc.stdout.strip():
        print(proc.stdout.strip())
    if proc.stderr.strip():
        print(proc.stderr.strip(), file=sys.stderr)

    stopped = subprocess.run(
        [
            "curl",
            "-fsS",
            "--unix-socket",
            str(SERVICE_SOCKET),
            "-X",
            "POST",
            f"http://localhost/vms/{session_id}/stop",
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    if stopped.returncode != 0:
        print(f"{RED}FAIL: could not stop doctor ledger session{RESET}")
        print((stopped.stdout + stopped.stderr)[-8_000:])
        sys.exit(stopped.returncode)

    print(f"  session: {CYAN}{session_id}{RESET}  exit_code: {exit_code}")
    return session_id, session_dir, exit_code


def verify_session(session_id: str, session_dir: Path) -> bool:
    """Validate the stopped session against its session and main ledgers."""
    return _verify_session(session_id, session_dir, main_db=MAIN_DB)


def main():
    parser = argparse.ArgumentParser(
        description="Validate session DB produced by capsem-doctor run.",
    )
    parser.add_argument(
        "--binary",
        default="cache/target/cargo/debug/capsem",
        help="Path to the capsem binary (default: cache/target/cargo/debug/capsem)",
    )
    parser.add_argument(
        "--assets",
        default="cache/target/assets",
        help="Path to VM assets directory (default: cache/target/assets)",
    )
    args = parser.parse_args()

    mock_proc = None
    try:
        mock_proc, ready = start_mock_server()
        mock_base_url = ready["base_url"]
        print(f"{BOLD}Local mock server:{RESET} {mock_base_url}")
        session_id, session_dir, exit_code = run_doctor(args.binary, args.assets, mock_base_url)
    finally:
        stop_process(mock_proc)

    # capsem-doctor must pass -- a failure is itself a test failure.
    if exit_code != 0:
        print(f"{RED}FAIL: capsem-doctor exited with code {exit_code}{RESET}")
        print("capsem-doctor must pass before session validation can proceed.")
        sys.exit(1)
    print(f"  {GREEN}PASS{RESET}  capsem-doctor exited with code 0")

    ok = verify_session(session_id, session_dir)
    if ok:
        deleted = subprocess.run(
            [args.binary, "delete", session_id],
            env=_cli_env(args.assets),
            capture_output=True,
            text=True,
            timeout=60,
        )
        if deleted.returncode != 0:
            print(f"{RED}FAIL: could not delete validated doctor session{RESET}")
            print((deleted.stdout + deleted.stderr)[-8_000:])
            ok = False
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
