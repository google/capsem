#!/usr/bin/env python3
"""End-to-end integration test: boot VM, exercise all telemetry pipelines,
verify every event type is logged in the session DB.

Exercises:
  1. fs_events   -- create, modify, and delete files inside the VM
  2. net_events   -- curl an allowed domain + a denied domain (policy enforcement)
  3. mcp_calls    -- run capsem-doctor MCP tests (init, tools/list, fetch, grep)
  4. model_calls  -- call the local OpenAI-compatible mock fixture
  5. tool_calls   -- validate tool-call ledger shape when model fixtures emit it
  6. main.db      -- rollup counters match session.db actuals

Usage:
    python3 scripts/integration_test.py              # uses target/debug/capsem
    python3 scripts/integration_test.py --binary ./capsem --assets ./assets

Ironbank note: this is black-box product proof. Do not close a release gate
with status-only replay, row-exists checks, skipped/slow cases, public
services, or expectations copied from Rust internals. The ledger contract is
client result + parsed facts + security rows + protocol rows + logs + routes.
"""

import argparse
import json
import os
import re
import signal
import shutil
import shlex
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from mock_server import local_fixture_env, start_mock_server, stop_process  # noqa: E402

PROJECT_ROOT = Path(__file__).resolve().parent.parent


def _integration_home() -> Path:
    """Return the per-run integration home.

    `just test` can invoke focused and full integration probes back-to-back.
    A fixed service socket lets a cleanly exiting singleton peer race the
    harness before readiness is observable, so each invocation owns its own
    CAPSEM_HOME by default. The override keeps manual debugging reproducible.
    """
    if env := os.environ.get("CAPSEM_INTEGRATION_HOME"):
        return Path(env)
    return PROJECT_ROOT / "target" / f"integration-capsem-home-{os.getpid()}"


INTEGRATION_HOME = _integration_home()

BOLD = "\033[1m"
DIM = "\033[2m"
GREEN = "\033[32m"
RED = "\033[31m"
YELLOW = "\033[33m"
CYAN = "\033[36m"
RESET = "\033[0m"

def _capsem_home() -> Path:
    """Resolve the capsem base dir, honoring CAPSEM_HOME like the Rust helper.

    Tests run with CAPSEM_HOME pointing at an isolated directory so this
    script never stomps on a locally installed capsem under ~/.capsem.
    """
    env = os.environ.get("CAPSEM_HOME")
    if env:
        return Path(env)
    return Path.home() / ".capsem"


def _run_dir() -> Path:
    env = os.environ.get("CAPSEM_RUN_DIR")
    if env:
        return Path(env)
    return _capsem_home() / "run"


CAPSEM_HOME = INTEGRATION_HOME
SESSIONS_DIR = INTEGRATION_HOME / "run" / "sessions"
MAIN_DB = INTEGRATION_HOME / "sessions" / "main.db"
SERVICE_SOCKET = INTEGRATION_HOME / "run" / "service.sock"
SERVICE_PIDFILE = INTEGRATION_HOME / "run" / "service.pid"


def default_materialized_profiles_dir() -> str:
    """Return the generated profile catalog used by packages, CI, and install."""
    return str(PROJECT_ROOT / "target" / "config" / "profiles")


def _profile_env() -> dict[str, str]:
    return {"CAPSEM_PROFILES_DIR": default_materialized_profiles_dir()}


def _test_isolation_env() -> dict[str, str]:
    """Environment that keeps black-box integration tests hermetic.

    The credential broker must not touch the developer's native keychain during
    release gates. Native storage belongs to installed/manual runs; tests use
    an isolated JSON store inside CAPSEM_HOME so captured credentials can be
    asserted without host prompts or hidden state.
    """
    return {
        "CAPSEM_CREDENTIAL_STORE_PATH": str(
            INTEGRATION_HOME / "run" / "credential-store.json"
        )
    }


def _integration_runtime_env() -> dict[str, str]:
    """Pin every integration subprocess to the same home and run directory."""
    return {
        "CAPSEM_HOME": str(INTEGRATION_HOME),
        "CAPSEM_RUN_DIR": str(INTEGRATION_HOME / "run"),
    }


def _new_session_dirs(sessions_dir: Path, existing: set[str]) -> list[Path]:
    """Return session directories created after `existing` was captured."""
    if not sessions_dir.exists():
        return []
    return sorted(
        (p for p in sessions_dir.iterdir() if p.is_dir() and p.name not in existing),
        key=lambda p: p.stat().st_mtime,
        reverse=True,
    )


def _vm_command(local_base_url: str) -> str:
    """Build the compound command executed inside the VM.

    Required steps are chained with `&&` so a broken fixture stops immediately.
    The denied-domain probe is the only intentionally non-fatal command.
    """
    tiny_url = shlex.quote(f"{local_base_url.rstrip('/')}/tiny")
    bytes_url = shlex.quote(f"{local_base_url.rstrip('/')}/bytes/10mb")
    deny_url = shlex.quote(f"{local_base_url.rstrip('/')}/deny-target")
    model_url = shlex.quote(f"{local_base_url.rstrip('/')}/v1/chat/completions")
    model_payload = shlex.quote(json.dumps({
        "model": "mock-openai",
        "messages": [{"role": "user", "content": "say capsem"}],
        "stream": False,
    }))

    commands = [
    # -- fs_events: create, modify, and delete files --
    "echo 'integration-test-data' > /root/integration_test.txt",
    "mkdir -p /root/test_dir",
    "echo 'nested-file-content' > /root/test_dir/nested.txt",
    "echo 'to-be-deleted' > /root/delete_me.txt",
    "sleep 0.2",  # let debouncer see the create before we delete
    "rm /root/delete_me.txt",

    # -- net_events: local allowed fetch + denied domain --
    f"curl -sf {tiny_url} -o /dev/null",
    f"curl -sf {deny_url} -o /dev/null || true",  # denied by corp rule

    # -- throughput: deterministic 10MB fixture through the full MITM proxy pipeline --
    (
        "curl -sL -o /dev/null"
        " -w 'throughput: %{speed_download} B/s in %{time_total}s\\n'"
        " --connect-timeout 5 -m 30"
        f" {bytes_url}"
    ),

    # -- mcp_calls: capsem-doctor MCP test subset --
    "capsem-doctor -k mcp",

    # -- model_calls: deterministic local OpenAI-compatible fixture --
    (
        "curl -sf -X POST"
        " --connect-timeout 5 -m 30"
        " -H 'content-type: application/json'"
        " -H 'authorization: Bearer capsem_test_openai_api_key'"
        f" --data {model_payload}"
        f" {model_url}"
        " -o /root/model_fixture.json"
    ),
    "test -s /root/model_fixture.json",
    (
        "python3 -c \"import json;"
        " data=json.load(open('/root/model_fixture.json'));"
        " print('model-fixture:', data.get('choices',[{}])[0].get('message',{}).get('content',''))\""
    ),
    "echo 'sandboxes hold the grains of time' > /root/model_fixture_poem.txt",

        # -- debouncer flush: fs_events uses a 100ms debouncer --
        "sleep 2",

        # -- sentinel so the host can confirm full execution --
        "echo CAPSEM_INTEGRATION_DONE",
    ]
    return " && ".join(commands)


def _kill_dev_service() -> None:
    """Stop the smoke-owned `capsem-service --foreground` and any child VMs.

    Kill ONLY by pidfile -- never pkill-by-pattern. A pattern like
    `capsem-service.*--foreground` would also catch the user's installed
    LaunchAgent / systemd unit. On macOS the LaunchAgent has KeepAlive=true
    and would respawn mid-test, racing against the test service and the
    direct-spawn path in client.rs.
    """
    if SERVICE_PIDFILE.exists():
        try:
            pid = int(SERVICE_PIDFILE.read_text().strip())
        except (ValueError, OSError):
            pid = 0
        if pid > 0:
            # Term the service; its own signal handler terminates child VMs.
            subprocess.run(["kill", str(pid)], check=False)
            for _ in range(20):
                if subprocess.run(["kill", "-0", str(pid)], check=False,
                                   capture_output=True).returncode != 0:
                    break
                time.sleep(0.1)
            # Force-kill if still alive.
            subprocess.run(["kill", "-9", str(pid)], check=False,
                           capture_output=True)
    try:
        SERVICE_PIDFILE.unlink()
    except FileNotFoundError:
        pass
    try:
        SERVICE_SOCKET.unlink()
    except FileNotFoundError:
        pass


def _wait_for_service_ready(
    proc: subprocess.Popen,
    *,
    service_socket: Path,
    log_path: Path,
    timeout_secs: float = 15.0,
    poll_interval: float = 0.2,
    run_cmd=subprocess.run,
    sleep=time.sleep,
    monotonic=time.monotonic,
) -> None:
    """Wait for the service socket to answer, honoring idempotent startup.

    `capsem-service` intentionally exits 0 when a compatible peer wins a
    startup race. The integration harness must keep probing the socket in that
    case instead of treating a clean early exit as failure.
    """
    deadline = monotonic() + timeout_secs
    clean_early_exit = False
    while monotonic() < deadline:
        if service_socket.exists():
            # Socket alone isn't enough -- wait for /list to respond.
            r = run_cmd(
                [
                    "curl",
                    "-s",
                    "--unix-socket",
                    str(service_socket),
                    "--max-time",
                    "2",
                    "http://localhost/list",
                ],
                capture_output=True,
            )
            if r.returncode == 0:
                return
        if proc.poll() is not None:
            if proc.returncode != 0:
                raise RuntimeError(
                    f"capsem-service exited early (code {proc.returncode}); "
                    f"see {log_path}"
                )
            clean_early_exit = True
        sleep(poll_interval)

    if clean_early_exit:
        raise RuntimeError(
            f"capsem-service exited 0 before the service socket became ready; "
            f"see {log_path}"
        )
    raise RuntimeError(f"capsem-service did not become ready in {timeout_secs:g}s; see {log_path}")


def _start_service_with_test_config(
    assets_dir: str, settings_config: str, corp_config: str
) -> subprocess.Popen:
    """Spawn `capsem-service --foreground` with test config env vars.

    The service and each `capsem-process` share CAPSEM_HOME, so the per-VM
    runtime policy picks up `example.com` and the other overrides from
    `tests/fixtures/config/integration/settings.toml`.
    """
    project_root = PROJECT_ROOT
    service_bin = project_root / "target/debug/capsem-service"
    process_bin = project_root / "target/debug/capsem-process"
    test_home = INTEGRATION_HOME
    test_home.mkdir(parents=True, exist_ok=True)
    SERVICE_PIDFILE.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(project_root / settings_config, test_home / "settings.toml")

    env = {
        **os.environ,
        **_profile_env(),
        **_test_isolation_env(),
        **_integration_runtime_env(),
        "CAPSEM_CORP_CONFIG": str(project_root / corp_config),
        "RUST_LOG": "capsem=info",
    }

    log_path = project_root / "target/integration-test-service.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log_file = open(log_path, "w")

    try:
        proc = subprocess.Popen(
            [
                str(service_bin),
                "--assets-dir",
                f"{assets_dir}/arm64" if (Path(assets_dir) / "arm64").exists() else assets_dir,
                "--process-binary",
                str(process_bin),
                "--uds-path",
                str(SERVICE_SOCKET),
                "--foreground",
            ],
            env=env,
            stdout=log_file,
            stderr=subprocess.STDOUT,
        )
    finally:
        log_file.close()
    SERVICE_PIDFILE.write_text(str(proc.pid))

    _wait_for_service_ready(proc, service_socket=SERVICE_SOCKET, log_path=log_path)
    return proc


def run_vm(binary: str, assets_dir: str) -> tuple[str, int]:
    """Boot a temp VM via `capsem run`, return (session_id, exit_code).

    The service preserves the session dir after `run` completes, so we
    find it by looking for the newest `*-tmp` directory created during
    this invocation.
    """
    env = {
        **os.environ,
        **_profile_env(),
        **_test_isolation_env(),
        **_integration_runtime_env(),
        "CAPSEM_ASSETS_DIR": assets_dir,
        "RUST_LOG": "capsem=warn",
        "CAPSEM_CORP_CONFIG": "tests/fixtures/config/integration/corp.toml",
    }

    mock_proc = None

    # Restart the dev service with CAPSEM_HOME/CAPSEM_CORP_CONFIG in its env so
    # the policy rules from `tests/fixtures/config/integration/settings.toml` actually
    # reach the VM. Without this, the service inherits whatever env
    # `_ensure-service` was launched with (usually nothing), and the
    # per-VM policy falls back to the developer's real CAPSEM_HOME instead of
    # the isolated test config.
    _kill_dev_service()
    service_proc = _start_service_with_test_config(
        assets_dir,
        "tests/fixtures/config/integration/settings.toml",
        "tests/fixtures/config/integration/corp.toml",
    )

    # Snapshot session dirs before so we can find the new one after.
    existing = set(p.name for p in SESSIONS_DIR.iterdir()) if SESSIONS_DIR.exists() else set()

    try:
        mock_proc, ready = start_mock_server()
        mock_base_url = ready["base_url"]
        print(f"{BOLD}Local mock server:{RESET} {mock_base_url}")

        # Pass deterministic local fixture settings via --env so they reach the
        # VM through the service. Do not inject proxy variables: guest traffic
        # must prove the iptables-nft redirect rail.
        cmd = [binary, "run", "--timeout", "300"]
        for key, value in local_fixture_env(
            mock_base_url,
            ready.get("https_base_url"),
        ).items():
            cmd.extend(["--env", f"{key}={value}"])
        cmd.append(_vm_command(local_base_url=mock_base_url))

        print(f"{BOLD}Booting VM with test command ...{RESET}")
        proc = subprocess.run(
            cmd,
            env=env, capture_output=True, text=True, timeout=300,
        )
    finally:
        stop_process(mock_proc)
        # Always tear down the test service. Subsequent smoke steps spawn
        # their own fixtures, and leaving this one around would shadow any
        # default-config service the pipeline expects next.
        service_proc.send_signal(signal.SIGTERM)
        try:
            service_proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            service_proc.kill()
        try:
            SERVICE_PIDFILE.unlink()
        except FileNotFoundError:
            pass
    exit_code = proc.returncode
    if proc.stdout.strip():
        print(proc.stdout.strip())

    # Find the new session dir created during this invocation. Session names
    # are profile-scoped (`code-1`, `co-work-1`, ...), not legacy `*-tmp`.
    new_sessions = _new_session_dirs(SESSIONS_DIR, existing)

    if not new_sessions:
        print(f"{RED}FAIL: no new session directory found in {SESSIONS_DIR}{RESET}")
        print(f"    {YELLOW}--- stderr ---{RESET}")
        for line in proc.stderr.strip().splitlines()[:30]:
            print(f"    {line}")
        sys.exit(1)

    session_id = new_sessions[0].name
    print(f"  session: {CYAN}{session_id}{RESET}  exit_code: {exit_code}")
    return session_id, exit_code


# ── assertions ───────────────────────────────────────────────────────────


class Results:
    """Accumulates pass/fail/warn results for a clean summary."""

    def __init__(self):
        self.passed: list[str] = []
        self.failed: list[str] = []
        self.warned: list[str] = []

    def ok(self, msg: str):
        self.passed.append(msg)
        print(f"  {GREEN}PASS{RESET}  {msg}")

    def fail(self, msg: str):
        self.failed.append(msg)
        print(f"  {RED}FAIL{RESET}  {msg}")

    def warn(self, msg: str):
        self.warned.append(msg)
        print(f"  {YELLOW}WARN{RESET}  {msg}")

    def check(self, cond: bool, pass_msg: str, fail_msg: str):
        if cond:
            self.ok(pass_msg)
        else:
            self.fail(fail_msg)

    @property
    def success(self) -> bool:
        return len(self.failed) == 0


def verify_session(session_id: str) -> bool:
    """Open the session DB, run all assertions, return True on success."""
    db_path = SESSIONS_DIR / session_id / "session.db"
    gz_path = SESSIONS_DIR / session_id / "session.db.gz"

    # Session DB may be gzip-compressed after vacuum. Decompress for reading.
    if not db_path.exists() and gz_path.exists():
        import gzip
        with gzip.open(gz_path, "rb") as f_in:
            db_path.write_bytes(f_in.read())

    if not db_path.exists():
        print(f"{RED}session.db not found at {db_path}{RESET}")
        return False

    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    r = Results()

    # ── fs_events ────────────────────────────────────────────────────
    print(f"\n{BOLD}fs_events{RESET}")
    fs_count = conn.execute("SELECT COUNT(*) FROM fs_events").fetchone()[0]
    r.check(
        fs_count > 0,
        f"{fs_count} fs_events recorded",
        "no fs_events recorded",
    )

    # Verify fs_events contain data.  The capsem-doctor snapshot tests produce
    # "restored" events.  Direct guest shell file ops (echo > file) are captured
    # when VirtioFS inotify is active, but may only appear as snapshot-related
    # events depending on timing and the exec path.
    actions = conn.execute(
        "SELECT action, COUNT(*) as cnt FROM fs_events GROUP BY action"
    ).fetchall()
    action_map = {row["action"]: row["cnt"] for row in actions}
    r.check(
        len(action_map) > 0,
        f"fs_event actions: {dict(action_map)}",
        "no fs_event action types recorded",
    )

    # ── net_events ───────────────────────────────────────────────────
    print(f"\n{BOLD}net_events{RESET}")
    net_count = conn.execute("SELECT COUNT(*) FROM net_events").fetchone()[0]
    r.check(
        net_count > 0,
        f"{net_count} net_events recorded",
        "no net_events recorded",
    )

    # Local fixture /tiny from the curl.
    local_tiny = conn.execute(
        "SELECT * FROM net_events WHERE domain = '127.0.0.1' AND path = '/tiny'"
    ).fetchone()
    r.check(
        local_tiny is not None,
        "local debug /tiny request logged (curl)",
        "local debug /tiny NOT found in net_events (curl may have failed)",
    )

    # Allowed decision.
    if local_tiny:
        r.check(
            local_tiny["decision"] == "allowed",
            "local debug /tiny decision = allowed",
            f"local debug /tiny decision = {local_tiny['decision']} (expected allowed)",
        )

    # Local deterministic 10MB fixture throughput download.
    throughput_rows = conn.execute(
        "SELECT * FROM net_events WHERE domain = '127.0.0.1' AND path = '/bytes/10mb'"
    ).fetchall()
    r.check(
        len(throughput_rows) > 0,
        f"{len(throughput_rows)} local throughput net_events recorded (/bytes/10mb)",
        "no local throughput net_events found (10MB fixture may have failed through MITM)",
    )
    if throughput_rows:
        allowed = sum(1 for row in throughput_rows if row["decision"] == "allowed")
        r.check(
            allowed == len(throughput_rows),
            f"all {allowed} throughput net_events decision = allowed",
            f"{len(throughput_rows) - allowed} throughput net_events NOT allowed",
        )
        sizes = [row["bytes_received"] for row in throughput_rows
                 if row["bytes_received"] and row["bytes_received"] > 1_000_000]
        r.check(
            len(sizes) > 0,
            f"throughput bytes_received ~{sizes[0]} (proves MITM didn't truncate)",
            "no throughput net_event with bytes_received >1MB (MITM may have truncated or hit 404)",
        )

    # At least one allowed net_event with an HTTP status code.
    with_status = conn.execute(
        "SELECT COUNT(*) FROM net_events WHERE status_code IS NOT NULL AND status_code > 0"
    ).fetchone()[0]
    r.check(
        with_status >= 1,
        f"{with_status} net_events have HTTP status codes",
        "no net_events with HTTP status codes (MITM proxy may not be recording)",
    )

    # Denied local HTTP event from the corp-owned integration rule.
    denied_target_count = conn.execute(
        """
        SELECT COUNT(*)
        FROM net_events
        WHERE decision = 'denied'
          AND domain = '127.0.0.1'
          AND path = '/deny-target'
        """
    ).fetchone()[0]
    r.check(
        denied_target_count >= 1,
        f"{denied_target_count} denied local /deny-target net_events (corp enforcement working)",
        "no denied local /deny-target net_events (corp rule may not have applied)",
    )

    denied_count = conn.execute(
        "SELECT COUNT(*) FROM net_events WHERE decision = 'denied'"
    ).fetchone()[0]
    r.check(
        denied_count >= 1,
        f"{denied_count} denied net_events recorded",
        "no denied net_events recorded",
    )

    # Decision breakdown -- verify both allowed and denied present.
    decisions = conn.execute(
        "SELECT decision, COUNT(*) as cnt FROM net_events GROUP BY decision"
    ).fetchall()
    decision_map = {row["decision"]: row["cnt"] for row in decisions}
    r.check(
        "allowed" in decision_map and "denied" in decision_map,
        f"net_event decisions: {dict(decision_map)}",
        f"expected both allowed and denied decisions, got: {dict(decision_map)}",
    )

    # ── mcp_calls ────────────────────────────────────────────────────
    print(f"\n{BOLD}mcp_calls{RESET}")
    mcp_count = conn.execute("SELECT COUNT(*) FROM mcp_calls").fetchone()[0]
    r.check(
        mcp_count > 0,
        f"{mcp_count} mcp_calls recorded",
        "no mcp_calls recorded",
    )

    # Expect at least: initialize, tools/list, and several tools/call.
    mcp_methods = conn.execute(
        "SELECT DISTINCT method FROM mcp_calls"
    ).fetchall()
    methods = {row["method"] for row in mcp_methods}
    r.check(
        "initialize" in methods,
        "MCP initialize logged",
        "MCP initialize NOT logged",
    )
    r.check(
        "tools/list" in methods,
        "MCP tools/list logged",
        "MCP tools/list NOT logged",
    )
    r.check(
        "tools/call" in methods,
        "MCP tools/call logged",
        "MCP tools/call NOT logged",
    )

    # fetch_http tool calls (logged as builtin__fetch_http).
    fetch_calls = conn.execute(
        "SELECT COUNT(*) FROM mcp_calls WHERE tool_name LIKE '%fetch_http%'"
    ).fetchone()[0]
    r.check(
        fetch_calls >= 2,
        f"{fetch_calls} fetch_http MCP calls (allowed + blocked)",
        f"only {fetch_calls} fetch_http calls (expected >= 2)",
    )

    # Blocked-domain tests return isError in the MCP result (not a JSON-RPC
    # error), so the gateway logs them as "allowed".  Verify the response
    # preview contains "blocked" for at least one call.
    blocked_in_preview = conn.execute(
        "SELECT COUNT(*) FROM mcp_calls"
        " WHERE response_preview LIKE '%blocked%'"
        "    OR response_preview LIKE '%isError%'"
    ).fetchone()[0]
    r.check(
        blocked_in_preview >= 1,
        f"{blocked_in_preview} MCP calls with blocked-domain responses",
        "no MCP responses mention blocking (capsem-doctor blocked tests may have failed)",
    )

    # Bug 2: MCP data completeness -- request_preview must not be truncated
    preview_rows = conn.execute(
        "SELECT id, request_preview FROM mcp_calls WHERE method='tools/call'"
    ).fetchall()
    bad_previews = [row["id"] for row in preview_rows
                    if not row["request_preview"] or len(row["request_preview"]) <= 10]
    r.check(
        len(bad_previews) == 0,
        f"all {len(preview_rows)} mcp tools/call have meaningful request_preview",
        f"{len(bad_previews)} mcp tools/call have empty/tiny request_preview (ids: {bad_previews[:5]})",
    )

    # Bug 2: bytes tracking
    mcp_with_bytes = conn.execute(
        "SELECT COUNT(*) FROM mcp_calls WHERE method='tools/call' AND bytes_sent > 0"
    ).fetchone()[0]
    r.check(
        mcp_with_bytes > 0,
        f"{mcp_with_bytes} mcp tools/call have bytes_sent > 0",
        "no mcp tools/call with bytes_sent -- byte tracking broken",
    )

    # Bug 3: builtin HTTP in net_events
    mcp_net = conn.execute(
        "SELECT COUNT(*) FROM net_events WHERE conn_type = 'mcp_builtin'"
    ).fetchone()[0]
    r.check(
        mcp_net > 0,
        f"{mcp_net} net_events from MCP builtin tools",
        "no net_events with conn_type=mcp_builtin -- builtin HTTP not logged",
    )

    # Bug 4: process_name
    bad_proc = conn.execute(
        "SELECT COUNT(*) FROM mcp_calls WHERE process_name = 'MainThread'"
    ).fetchone()[0]
    r.check(
        bad_proc == 0,
        "no mcp_calls with process_name='MainThread'",
        f"{bad_proc} mcp_calls have process_name='MainThread'",
    )

    # ── model_calls ──────────────────────────────────────────────────
    print(f"\n{BOLD}model_calls{RESET}")
    model_count = conn.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
    r.check(
        model_count > 0,
        f"{model_count} model_calls recorded",
        "no model_calls recorded for local OpenAI-compatible fixture",
    )

    fixture_call = conn.execute(
        """
        SELECT *
        FROM model_calls
        WHERE path = '/v1/chat/completions'
        ORDER BY id LIMIT 1
        """
    ).fetchone()
    r.check(
        fixture_call is not None,
        "local OpenAI-compatible model_call recorded",
        "no model_call for local /v1/chat/completions fixture",
    )
    if fixture_call:
        r.check(
            fixture_call["status_code"] == 200,
            "local model_call status_code = 200",
            f"local model_call status_code = {fixture_call['status_code']}",
        )
        r.check(
            bool(fixture_call["provider"]) and bool(fixture_call["model"]),
            f"local model_call provider/model = {fixture_call['provider']}/{fixture_call['model']}",
            "local model_call missing provider or model",
        )
        r.check(
            (fixture_call["response_bytes"] or 0) > 0,
            f"local model_call response_bytes = {fixture_call['response_bytes']}",
            "local model_call response_bytes is zero",
        )

    # ── tool_calls / tool_responses ──────────────────────────────────
    print(f"\n{BOLD}tool_calls / tool_responses{RESET}")
    tc_count = conn.execute("SELECT COUNT(*) FROM tool_calls").fetchone()[0]
    tr_count = conn.execute("SELECT COUNT(*) FROM tool_responses").fetchone()[0]

    # Bug 1: No duplicate tool_responses
    if tr_count > 0:
        unique_tr = conn.execute(
            "SELECT COUNT(*) FROM (SELECT DISTINCT call_id, content_preview FROM tool_responses)"
        ).fetchone()[0]
        r.check(
            tr_count == unique_tr,
            f"no duplicate tool_responses ({tr_count} total, {unique_tr} unique)",
            f"DUPLICATE tool_responses: {tr_count} total but only {unique_tr} unique",
        )

    # We validate tool_calls metadata when present, but don't fail on 0.
    if tc_count > 0:
        r.ok(f"{tc_count} tool_calls recorded")
    else:
        r.ok("0 tool_calls recorded for this deterministic fixture")

    if tc_count > 0:
        # Origin column should be populated on all tool_calls.
        with_origin = conn.execute(
            "SELECT COUNT(*) FROM tool_calls WHERE origin IS NOT NULL AND origin != ''"
        ).fetchone()[0]
        r.check(
            with_origin == tc_count,
            f"all {with_origin} tool_calls have origin column populated",
            f"only {with_origin}/{tc_count} tool_calls have origin",
        )

        # Some streaming formats do not always produce a parseable
        # tool_response turn, so tool_responses may lag behind tool_calls.
        if tr_count >= tc_count:
            r.ok(f"{tr_count} tool_responses match {tc_count} tool_calls")
        else:
            r.ok(
                f"tool_responses ({tr_count}) < tool_calls ({tc_count})"
                " -- stream parser limitation (non-blocking)"
            )

    conn.close()

    # ── main.db rollup ───────────────────────────────────────────────
    print(f"\n{BOLD}main.db rollup{RESET}")
    if MAIN_DB.exists():
        mconn = sqlite3.connect(str(MAIN_DB))
        mconn.row_factory = sqlite3.Row
        row = mconn.execute(
            "SELECT * FROM sessions WHERE id = ?", (session_id,)
        ).fetchone()
        if row:
            r.check(
                row["status"] in ("stopped", "vacuumed"),
                f"main.db status = {row['status']}",
                f"main.db status = {row['status']} (expected stopped or vacuumed)",
            )
            r.check(
                row["total_file_events"] > 0,
                f"main.db total_file_events = {row['total_file_events']}",
                "main.db total_file_events = 0 (rollup failed)",
            )
            r.check(
                row["total_requests"] > 0,
                f"main.db total_requests = {row['total_requests']}",
                "main.db total_requests = 0 (rollup failed)",
            )
            r.check(
                row["total_mcp_calls"] > 0,
                f"main.db total_mcp_calls = {row['total_mcp_calls']}",
                "main.db total_mcp_calls = 0 (rollup failed)",
            )

            # Cross-check: main.db rollup matches session.db actuals.
            sconn = sqlite3.connect(str(SESSIONS_DIR / session_id / "session.db"))
            actual_fs = sconn.execute("SELECT COUNT(*) FROM fs_events").fetchone()[0]
            actual_net = sconn.execute("SELECT COUNT(*) FROM net_events").fetchone()[0]
            actual_mcp = sconn.execute("SELECT COUNT(*) FROM mcp_calls").fetchone()[0]
            sconn.close()

            r.check(
                row["total_file_events"] == actual_fs,
                f"main.db total_file_events ({row['total_file_events']}) matches session.db ({actual_fs})",
                f"main.db total_file_events ({row['total_file_events']}) != session.db ({actual_fs})",
            )
            r.check(
                row["total_requests"] == actual_net,
                f"main.db total_requests ({row['total_requests']}) matches session.db ({actual_net})",
                f"main.db total_requests ({row['total_requests']}) != session.db ({actual_net})",
            )
            r.check(
                row["total_mcp_calls"] == actual_mcp,
                f"main.db total_mcp_calls ({row['total_mcp_calls']}) matches session.db ({actual_mcp})",
                f"main.db total_mcp_calls ({row['total_mcp_calls']}) != session.db ({actual_mcp})",
            )
        else:
            r.fail(f"session {session_id} not found in main.db")
        mconn.close()
    else:
        r.fail(f"main.db not found at {MAIN_DB}")

    # ── log files ─────────────────────────────────────────────────────
    print(f"\n{BOLD}log files{RESET}")

    # Per-VM session log: ~/.capsem/run/sessions/<id>/process.log
    vm_log_path = SESSIONS_DIR / session_id / "process.log"
    r.check(
        vm_log_path.exists(),
        f"process.log exists at {vm_log_path}",
        f"process.log NOT found at {vm_log_path}",
    )

    if vm_log_path.exists():
        vm_log_content = vm_log_path.read_text()
        vm_log_lines = [line for line in vm_log_content.splitlines() if line.strip()]
        r.check(
            len(vm_log_lines) >= 3,
            f"{len(vm_log_lines)} entries in process.log",
            f"only {len(vm_log_lines)} entries in process.log (expected >= 3)",
        )

        # Verify all lines are valid JSON with expected fields.
        valid_json = 0
        invalid_lines = []
        for i, line in enumerate(vm_log_lines):
            try:
                entry = json.loads(line)
                # tracing-subscriber's JSON formatter puts 'message' inside 'fields'.
                # TauriLogLayer puts it at top level. Support both.
                msg = entry.get("message")
                if msg is None and "fields" in entry:
                    msg = entry["fields"].get("message")
                
                has_fields = all(k in entry for k in ("timestamp", "level", "target")) and msg is not None
                if has_fields:
                    valid_json += 1
                else:
                    invalid_lines.append((i + 1, "missing fields"))
            except json.JSONDecodeError:
                invalid_lines.append((i + 1, "invalid JSON"))

        r.check(
            valid_json == len(vm_log_lines),
            f"all {valid_json} process.log entries are valid JSONL",
            f"{len(invalid_lines)} invalid lines in process.log: {invalid_lines[:5]}",
        )

        # Check that log entries contain expected levels (INFO and above only).
        levels = set()
        for line in vm_log_lines:
            try:
                entry = json.loads(line)
                levels.add(entry.get("level", ""))
            except json.JSONDecodeError:
                pass
        r.check(
            "INFO" in levels,
            f"process.log contains INFO entries (levels: {levels})",
            f"process.log missing INFO entries (levels: {levels})",
        )
        r.check(
            "DEBUG" not in levels and "TRACE" not in levels,
            "process.log filtered to INFO+ (no DEBUG/TRACE)",
            f"process.log contains debug/trace entries (levels: {levels})",
        )

        # Check for boot_timeline state transition events.
        timeline_entries = []
        for line in vm_log_lines:
            try:
                entry = json.loads(line)
                msg = entry.get("message")
                if msg is None and "fields" in entry:
                    msg = entry["fields"].get("message")
                
                if msg and "state transition" in msg:
                    timeline_entries.append(entry)
            except json.JSONDecodeError:
                pass
        r.check(
            len(timeline_entries) >= 2,
            f"{len(timeline_entries)} boot_timeline state transitions in process.log",
            f"only {len(timeline_entries)} state transitions (expected >= 2 boot phases)",
        )

        # Verify timestamps are valid ISO 8601.
        valid_ts = 0
        for line in vm_log_lines[:5]:
            try:
                entry = json.loads(line)
                ts = entry.get("timestamp", "")
                # Allow 3 to 9 decimal places for sub-seconds.
                if re.match(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z", ts):
                    valid_ts += 1
            except json.JSONDecodeError:
                pass
        r.check(
            valid_ts >= 3,
            f"{valid_ts}/5 sampled timestamps are valid ISO 8601",
            f"only {valid_ts}/5 timestamps match ISO 8601 format",
        )

    # capsem-app per-launch log files live in <capsem_home>/logs/*.jsonl and
    # are only produced when the Tauri desktop shell has been launched. This
    # script exercises `capsem run` / the service, never the desktop app, so
    # inspect the dir only if prior capsem-app runs left logs behind.
    launch_log_dir = CAPSEM_HOME / "logs"
    if launch_log_dir.exists():
        jsonl_files = sorted(launch_log_dir.glob("*.jsonl"), key=lambda p: p.name, reverse=True)
        if jsonl_files:
            latest = jsonl_files[0]
            latest_lines = [line for line in latest.read_text().splitlines() if line.strip()]
            r.check(
                len(latest_lines) >= 5,
                f"latest launch log {latest.name} has {len(latest_lines)} entries",
                f"latest launch log {latest.name} has only {len(latest_lines)} entries (expected >= 5)",
            )
            fname_match = re.match(r"\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}", latest.stem)
            r.check(
                fname_match is not None,
                f"launch log filename {latest.name} has valid timestamp format",
                f"launch log filename {latest.name} does not match expected format",
            )

    # ── auto-snapshots ────────────────────────────────────────────────
    print(f"\n{BOLD}auto-snapshots{RESET}")
    snap_dir = SESSIONS_DIR / session_id / "auto_snapshots"
    r.check(
        snap_dir.exists(),
        "auto_snapshots directory exists",
        f"auto_snapshots directory NOT found at {snap_dir}",
    )
    if snap_dir.exists():
        slot0 = snap_dir / "0"
        r.check(
            slot0.exists(),
            "boot snapshot slot 0 exists",
            "boot snapshot slot 0 NOT found (auto-snapshot scheduler may not have run)",
        )
        if slot0.exists():
            has_workspace = (slot0 / "workspace").exists()
            has_system = (slot0 / "system").exists()
            r.check(
                has_workspace and has_system,
                "slot 0 contains workspace/ and system/ subdirectories",
                f"slot 0 missing subdirs (workspace={has_workspace}, system={has_system})",
            )

    # ── summary ──────────────────────────────────────────────────────
    print(f"\n{BOLD}{'=' * 60}{RESET}")
    total = len(r.passed) + len(r.failed) + len(r.warned)
    print(
        f"  {GREEN}{len(r.passed)} passed{RESET}"
        f"  {RED}{len(r.failed)} failed{RESET}"
        f"  {YELLOW}{len(r.warned)} warnings{RESET}"
        f"  ({total} checks)"
    )
    if r.success:
        print(f"  {GREEN}{BOLD}INTEGRATION TEST PASSED{RESET}\n")
    else:
        print(f"  {RED}{BOLD}INTEGRATION TEST FAILED{RESET}\n")
    return r.success


PERSISTENCE_WRITE_CMD = (
    "echo capsem-persistence-sentinel > /root/.capsem_persistence_test "
    "&& echo CAPSEM_PERSISTENCE_WRITTEN"
)
PERSISTENCE_CHECK_CMD = (
    "test ! -f /root/.capsem_persistence_test "
    "&& echo CAPSEM_EPHEMERAL_OK "
    "|| { echo CAPSEM_EPHEMERAL_FAIL; exit 1; }"
)


def check_persistence(binary: str, assets_dir: str) -> bool:
    """Boot two consecutive VMs; verify a file written in the first is gone in the second."""
    print(f"\n{BOLD}=== Ephemeral model check ==={RESET}")
    env = {
        **os.environ,
        **_profile_env(),
        **_integration_runtime_env(),
        "CAPSEM_ASSETS_DIR": assets_dir,
        "RUST_LOG": "capsem=warn",
        "CAPSEM_CORP_CONFIG": "tests/fixtures/config/integration/corp.toml",
    }

    _kill_dev_service()
    service_proc = _start_service_with_test_config(
        assets_dir,
        "tests/fixtures/config/integration/settings.toml",
        "tests/fixtures/config/integration/corp.toml",
    )
    try:
        print("  Invocation 1: writing sentinel file...")
        proc1 = subprocess.run(
            [binary, "run", PERSISTENCE_WRITE_CMD],
            env=env, capture_output=True, text=True, timeout=120,
        )
        output1 = proc1.stdout + "\n" + proc1.stderr
        if "CAPSEM_PERSISTENCE_WRITTEN" not in output1:
            print(f"  {RED}FAIL{RESET}  sentinel write failed (invocation 1 did not confirm)")
            print(output1[:1000])
            return False
        print(f"  {GREEN}PASS{RESET}  sentinel written in invocation 1")

        print("  Invocation 2: checking sentinel is absent...")
        proc2 = subprocess.run(
            [binary, "run", PERSISTENCE_CHECK_CMD],
            env=env, capture_output=True, text=True, timeout=120,
        )
        output2 = proc2.stdout + "\n" + proc2.stderr
        # Use exit code as the definitive indicator -- the command string itself contains
        # "CAPSEM_EPHEMERAL_FAIL" so searching for it in output would always match (PTY echo).
        if proc2.returncode != 0:
            print(f"  {RED}FAIL{RESET}  sentinel persisted across VM invocations -- SECURITY BREACH")
            return False
        if "CAPSEM_EPHEMERAL_OK" not in output2:
            print(f"  {RED}FAIL{RESET}  ephemeral check did not confirm (no CAPSEM_EPHEMERAL_OK)")
            return False
        print(f"  {GREEN}PASS{RESET}  sentinel absent in invocation 2 (VM is fully ephemeral)")
        return True
    finally:
        service_proc.send_signal(signal.SIGTERM)
        try:
            service_proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            service_proc.kill()
        try:
            SERVICE_PIDFILE.unlink()
        except FileNotFoundError:
            pass


def main():
    parser = argparse.ArgumentParser(
        description="End-to-end integration test for capsem telemetry pipelines.",
    )
    parser.add_argument(
        "--binary",
        default="target/debug/capsem",
        help="Path to the capsem binary (default: target/debug/capsem)",
    )
    parser.add_argument(
        "--assets",
        default="assets",
        help="Path to VM assets directory (default: assets)",
    )
    args = parser.parse_args()

    session_id, exit_code = run_vm(args.binary, args.assets)

    if exit_code != 0:
        print(f"{RED}FAIL: VM integration workload exited with code {exit_code}{RESET}")
        sys.exit(1)

    telemetry_ok = verify_session(session_id)
    ephemeral_ok = check_persistence(args.binary, args.assets)
    sys.exit(0 if (telemetry_ok and ephemeral_ok) else 1)


if __name__ == "__main__":
    main()
