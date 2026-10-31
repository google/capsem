"""Execute the production audit startup section with isolated audit transport.

The command fixture supplies kernel status replies and configuration failures;
native VM qualification separately proves real audit registration and recording.
"""

import shlex
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest
from helpers.bounded import bounded

ROOT = Path(__file__).resolve().parents[3]


@pytest.fixture
def audit_startup(tmp_path):
    root = tmp_path / "newroot"
    daemon = root / "usr/sbin/auditd"
    daemon.parent.mkdir(parents=True)
    daemon.write_text("#!/bin/sh\nexit 0\n")
    daemon.chmod(0o755)
    commands = tmp_path / "commands"
    commands.mkdir()
    calls = tmp_path / "calls"
    status_count = tmp_path / "status-count"
    # auditd starts in the background. Its transport can overlap auditctl:
    # protect both the read/modify/write counter and each complete NUL frame.
    # Shell noclobber provides atomic creation without a host utility's
    # existence-check race when another caller releases its directory lock.
    lock_record = """while ! (set -C; : > "$AUDIT_CALL_LOCK") 2>/dev/null; do /bin/sleep 0.001; done
trap 'rm -f "$AUDIT_CALL_LOCK"' 0
"""
    unlock_record = 'rm -f "$AUDIT_CALL_LOCK"\ntrap - 0\n'
    chroot = commands / "chroot"
    chroot.write_text(
        """#!/bin/sh
shift
""" + lock_record + """
count=0
if [ -f "$STATUS_COUNT" ]; then read -r count < "$STATUS_COUNT"; fi
if [ "$1" = /usr/sbin/auditctl ] && [ "$2" = -s ]; then
    count=$((count + 1))
    printf '%s\\n' "$count" > "$STATUS_COUNT"
fi
printf '%s\\0' "$count" "$#" "$@" >> "$AUDIT_CALLS"
""" + unlock_record + """
if [ "$1" = /usr/sbin/auditd ]; then exit 0; fi
if [ "$1" = /usr/sbin/auditctl ] && [ "$2" = -s ]; then
    if [ -n "${STATUS_REPLY:-}" ]; then
        printf '%s\\n' "$STATUS_REPLY"
    elif [ "$count" -gt "$READY_AFTER" ]; then
        printf 'pid 571\\n'
    else
        printf 'pid 0\\n'
    fi
    exit "${STATUS_EXIT:-0}"
fi
[ "$count" -gt "$READY_AFTER" ] || exit 1
[ "$2" != "${FAIL_COMMAND:-}" ]
"""
    )
    chroot.chmod(0o755)
    sleep = commands / "sleep"
    sleep.write_text(
        "#!/bin/sh\n"
        + lock_record
        + "printf '%s\\0' sleep \"$#\" \"$@\" >> \"$AUDIT_CALLS\"\n"
        + unlock_record
        + 'exec /bin/sleep "$@"\n'
    )
    sleep.chmod(0o755)
    source = (ROOT / "guest/artifacts/capsem-init").read_text()
    section = source.split("# Start kernel audit subsystem for execve recording.\n", 1)[1]
    section = section.split('boot_mark "audit"', 1)[0]
    section = section.replace("/newroot", shlex.quote(str(root)))

    def run(*, concurrent_queries=0, **overrides):
        environment = {
            "PATH": f"{commands}:/usr/local/bin:/usr/bin:/bin",
            "AUDIT_CALLS": str(calls),
            "STATUS_COUNT": str(status_count),
            "AUDIT_CALL_LOCK": str(tmp_path / "record.lock"),
            "READY_AFTER": "0",
            **overrides,
        }
        if concurrent_queries:
            def query(_index):
                return subprocess.run(
                    [str(chroot), str(root), "/usr/sbin/auditctl", "-s"],
                    env=environment, stdin=subprocess.DEVNULL,
                    text=True, capture_output=True, check=True, timeout=10,
                )
            with ThreadPoolExecutor(max_workers=16) as pool:
                list(pool.map(query, range(concurrent_queries)))
        result = subprocess.run(
            bounded(["/bin/sh", "-c", section], 15, env=environment),
            text=True, capture_output=True, check=False,
        )
        # NUL framing preserves arbitrary argv without starting Python for each
        # transport call. Real sleeps and the production retry budget stay intact.
        fields = iter(calls.read_text().split("\0")[:-1])
        recorded = []
        for kind in fields:
            arguments = [next(fields) for _ in range(int(next(fields)))]
            recorded.append(
                {"sleep": arguments} if kind == "sleep"
                else {"args": arguments, "status_count": int(kind)}
            )
        return result, recorded

    return run


def test_concurrent_transport_records_keep_complete_frames_and_status_counts(audit_startup):
    result, calls = audit_startup(concurrent_queries=64)
    assert result.returncode == 0, result.stderr
    queries = [row for row in calls if row.get("args") == ["/usr/sbin/auditctl", "-s"]]
    assert len(queries) == 65, calls
    assert [row["status_count"] for row in queries] == list(range(1, 66)), calls


@pytest.mark.parametrize("ready_after", [0, 2])
def test_registered_daemon_precedes_rules_and_lock_without_a_fixed_pause(audit_startup, ready_after):
    result, calls = audit_startup(READY_AFTER=str(ready_after))
    assert result.returncode == 0, result.stderr
    queries = [row for row in calls if row.get("args") == ["/usr/sbin/auditctl", "-s"]]
    assert len(queries) == ready_after + 1, calls
    config = [row for row in calls if row.get("args", [None, None])[1] in {"-a", "-e"}]
    assert [row["args"] for row in config] == [
        ["/usr/sbin/auditctl", "-a", "always,exit", "-F", "arch=b64", "-S", "execve", "-k", "capsem_exec"],
        ["/usr/sbin/auditctl", "-e", "2"],
    ]
    assert all(row["status_count"] > ready_after for row in config), calls
    assert len([row for row in calls if "sleep" in row]) == ready_after, calls
    assert "auditd started, rules locked" in result.stdout


@pytest.mark.parametrize("reply,exit_code", [("pid 0", 0), ("pid nope", 0), ("pid -1", 0), ("pid 7 extra", 0), ("enabled 2", 0), ("pid 7", 1)])
def test_unregistered_or_failed_status_cannot_claim_auditing_ready(audit_startup, reply, exit_code):
    result, calls = audit_startup(STATUS_REPLY=reply, STATUS_EXIT=str(exit_code))
    assert result.returncode != 0
    assert "FATAL" in result.stderr
    assert "auditd started, rules locked" not in result.stdout
    assert not any(row.get("args", [None, None])[1] in {"-a", "-e"} for row in calls)


@pytest.mark.parametrize("command", ["-a", "-e"])
def test_failed_rule_or_lock_stops_audit_startup(audit_startup, command):
    result, calls = audit_startup(FAIL_COMMAND=command)
    assert result.returncode != 0
    assert "FATAL" in result.stderr
    assert "auditd started, rules locked" not in result.stdout
    if command == "-a":
        assert not any(row.get("args") == ["/usr/sbin/auditctl", "-e", "2"] for row in calls)
