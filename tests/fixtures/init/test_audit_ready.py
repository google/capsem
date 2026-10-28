"""Execute the production audit startup section with isolated audit transport.

The command fixture supplies kernel status replies and configuration failures;
native VM qualification separately proves real audit registration and recording.
"""

import shlex
import subprocess
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
    chroot = commands / "chroot"
    chroot.write_text(
        """#!/bin/sh
shift
count=0
if [ -f "$STATUS_COUNT" ]; then read -r count < "$STATUS_COUNT"; fi
if [ "$1" = /usr/sbin/auditctl ] && [ "$2" = -s ]; then
    count=$((count + 1))
    printf '%s\\n' "$count" > "$STATUS_COUNT"
fi
printf '%s\\0' "$count" "$#" "$@" >> "$AUDIT_CALLS"
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
        "printf '%s\\0' sleep \"$#\" \"$@\" >> \"$AUDIT_CALLS\"\n"
        'exec /bin/sleep "$@"\n'
    )
    sleep.chmod(0o755)
    source = (ROOT / "guest/artifacts/capsem-init").read_text()
    section = source.split("# Start kernel audit subsystem for execve recording.\n", 1)[1]
    section = section.split('boot_mark "audit"', 1)[0]
    section = section.replace("/newroot", shlex.quote(str(root)))

    def run(**overrides):
        environment = {
            "PATH": f"{commands}:/usr/local/bin:/usr/bin:/bin",
            "AUDIT_CALLS": str(calls),
            "STATUS_COUNT": str(status_count),
            "READY_AFTER": "0",
            **overrides,
        }
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
