"""Network measurements consume the current typed exec response."""

import runpy
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]


class Client:
    def __init__(self, response):
        self.response = response
        self.calls = []

    def post(self, path, body, *, timeout):
        self.calls.append((path, body, timeout))
        return self.response


def guest():
    return runpy.run_path(str(ROOT / "benchmarks/collectors/network"))["guest"]


def test_network_reads_current_text_output():
    client = Client({"exit_code": 0, "stdout": {"encoding": "utf8", "data": "Tcp: 1 2\n"}})
    assert guest()(client, "vm-id", "cat /proc/net/snmp", timeout=7) == "Tcp: 1 2\n"
    assert client.calls == [
        ("/vms/vm-id/exec", {"command": "cat /proc/net/snmp", "timeout_secs": 7}, 12)
    ]


def test_network_rejects_binary_output():
    client = Client({"exit_code": 0, "stdout": {"encoding": "base64", "data": "AA=="}})
    with pytest.raises(AssertionError):
        guest()(client, "vm-id", "command")


@pytest.mark.parametrize("response", [None, {"exit_code": 1}, {"stdout": "no status"}])
def test_network_rejects_failed_exec(response):
    with pytest.raises(RuntimeError):
        guest()(Client(response), "vm-id", "command")
