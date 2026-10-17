"""Execute the production audit startup section with isolated audit transport.

The command fixture supplies kernel status replies and configuration failures;
native VM qualification separately proves real audit registration and recording.
"""

import json
import shlex
import subprocess
import sys
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
    calls = tmp_path / "calls.jsonl"
    status_count = tmp_path / "status-count"
    chroot = commands / "chroot"
    chroot.write_text(
        f"#!{sys.executable}\n"
        "import json, os, sys\n"
        "from pathlib import Path\n"
        "args = sys.argv[2:]\n"
        "counter = Path(os.environ['STATUS_COUNT'])\n"
        "count = int(counter.read_text()) if counter.exists() else 0\n"
        "if args == ['/usr/sbin/auditctl', '-s']:\n"
        "    count += 1\n"
        "    counter.write_text(str(count))\n"
        "with Path(os.environ['AUDIT_CALLS']).open('a') as output:\n"
        "    output.write(json.dumps({'args': args, 'status_count': count}) + '\\n')\n"
        "if args[0] == '/usr/sbin/auditd':\n"
        "    raise SystemExit(0)\n"
        "if args == ['/usr/sbin/auditctl', '-s']:\n"
        "    if os.environ.get('STATUS_REPLY'):\n"
        "        print(os.environ['STATUS_REPLY'])\n"
        "    else:\n"
        "        print('pid ' + ('571' if count > int(os.environ['READY_AFTER']) else '0'))\n"
        "    raise SystemExit(int(os.environ.get('STATUS_EXIT', '0')))\n"
        "if count <= int(os.environ['READY_AFTER']):\n"
        "    raise SystemExit(1)\n"
        "raise SystemExit(1 if args[1] == os.environ.get('FAIL_COMMAND') else 0)\n"
    )
    chroot.chmod(0o755)
    sleep = commands / "sleep"
    sleep.write_text(
        f"#!{sys.executable}\n"
        "import json, os, sys\n"
        "from pathlib import Path\n"
        "with Path(os.environ['AUDIT_CALLS']).open('a') as output:\n"
        "    output.write(json.dumps({'sleep': sys.argv[1:]}) + '\\n')\n"
        "os.execv('/bin/sleep', ['/bin/sleep', *sys.argv[1:]])\n"
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
        recorded = [json.loads(line) for line in calls.read_text().splitlines()]
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
