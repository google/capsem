"""capsem-debug runs on the minimal runtime, and its tools do their jobs there.

The suites that need test runners, network tools, package managers, model SDKs
or agent CLIs reach them through a capsem-debug session. This is the proof
that the session is what they assume: the pinned digest, run as the image's
unprivileged user in the workload's user namespace, every tool working rather
than merely present -- and none of it in the VM beside it.
"""

import shlex

import pytest
from helpers.debug_session import WORKSPACE, debug_session
from helpers.service import exec_output_text

from tests.fixtures.oci import debug_image
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]

pytestmark = pytest.mark.integration

AGENTS = {
    "claude --version": "2.1.229 (Claude Code)",
    "codex --version": "codex-cli 0.147.0",
    "gemini --version": "0.55.1",
    "agy --version": "1.1.3",
}

PROBE = """\
set -eu
cd "$WORKSPACE"
printf 'def test_runs():\\n    assert 6 * 7 == 42\\n' > test_probe.py
pytest -q -p no:cacheprovider test_probe.py | tail -1 | sed 's/^/PYTEST /'
node -e 'console.log("NODE " + [1, 2, 3].map((n) => n * 2).join(","))'
python3 -c 'import anthropic, litellm, ollama, openai; print("SDKS", openai.__version__, anthropic.__version__)'
printf 'capsem-debug bytes\\n' > payload
zstd -q -f payload -o payload.zst && zstd -q -d -c payload.zst | cmp - payload && echo ZSTD roundtrip
uv venv -q --python /usr/bin/python3 uvvenv && uvvenv/bin/python -c 'print("UV venv")'
python3 -m venv pyvenv && pyvenv/bin/python -c 'import sys; print("VENV", sys.prefix != sys.base_prefix)'
iperf3 -s -1 -D -p 5201 && sleep 0.5
iperf3 -c 127.0.0.1 -p 5201 -t 1 -J > iperf.json
python3 -c 'import json; print("IPERF", int(json.load(open("iperf.json"))["end"]["sum_received"]["bytes"]) > 0)'
python3 -m http.server 8080 --bind 127.0.0.1 >/dev/null 2>&1 &
server=$!
sleep 1
wrk -t1 -c1 -d1s http://127.0.0.1:8080/ | sed -n 's/^Requests\\/sec: *\\([0-9.]*\\).*/WRK \\1/p'
kill "$server"
dig -v 2>&1 | head -1 | sed 's/^/DIG /'
echo "TOOLS $(command -v nc socat ping jq sqlite3 git strace redis-benchmark curl wget | wc -l)"
"""


def run(client, vm_id, command, *, target=None, timeout=120):
    body = {"command": command, "timeout_secs": timeout}
    if target is not None:
        body["target"] = target
    result = client.post(f"/vms/{vm_id}/exec", body, timeout=timeout + 30)
    return result, exec_output_text(result)


def lines(output):
    return {line.split(" ", 1)[0]: line.split(" ", 1)[1] for line in output.splitlines() if " " in line}


def test_the_debug_image_runs_by_its_pin_and_its_tools_work(service, tmp_path):
    client = service.client()
    with debug_session(service, tmp_path / "registry", "debug-image") as vm_id:
        status = client.get(f"/vms/{vm_id}/container")
        assert status["state"] == "running", status
        assert status["digest"] == debug_image.pinned(), status

        result, identity = run(client, vm_id, "id -u; cat /proc/self/uid_map; pwd")
        assert result["exit_code"] == 0, result
        uid, mapping, cwd = identity.splitlines()
        assert uid == "1000"
        assert mapping.split() == ["0", "100000", "65536"], mapping
        assert cwd == WORKSPACE

        result, output = run(client, vm_id, f"WORKSPACE={WORKSPACE} sh -c {shlex.quote(PROBE)}")
        (tmp_path / "probe.txt").write_text(output)
        assert result["exit_code"] == 0, output
        found = lines(output)
        assert found["PYTEST"].startswith("1 passed"), output
        assert found["UV"] == "venv", output
        assert found["NODE"] == "2,4,6", output
        assert found["SDKS"].split()[0] == "2.54.0", output
        assert found["ZSTD"] == "roundtrip", output
        assert found["VENV"] == "True", output
        assert found["IPERF"] == "True", output
        assert float(found["WRK"]) > 0, output
        assert found["DIG"].startswith("DiG 9."), output
        assert found["TOOLS"] == "10", output

        for command, version in AGENTS.items():
            result, output = run(client, vm_id, command)
            assert result["exit_code"] == 0, (command, output)
            assert version in output, (command, output)

        # None of it is in the VM: the runtime carries no test tooling.
        result, output = run(
            client,
            vm_id,
            "for t in node npm uv iperf3 wrk dig claude codex; do command -v $t || true; done",
            target="vm",
        )
        assert result["exit_code"] == 0, result
        assert output.strip() == "", output
