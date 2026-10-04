"""Capability groups: what an image's qualify.toml says it ships, proven.

A group runs only for an image that declares its capability, and then it is
required. Each uses the image's own binaries in its own workload; nothing is
installed for the test.
"""

import pytest
from helpers.image_session import WORKSPACE, workload_exec

pytestmark = pytest.mark.integration


def requires(candidate, capability):
    if capability not in candidate.capabilities:
        pytest.skip(f"{candidate.name} does not declare {capability}")


def test_toolchain_builds_and_runs_a_program(service, candidate, session):
    requires(candidate, "toolchain")
    result = workload_exec(
        service.client(),
        session,
        f"cd {WORKSPACE} && printf 'int main(void){{return 42;}}\\n' > t.c && cc -o t t.c; ./t; echo $?; "
        "python3 -c 'print(6*7)'; node -e 'console.log(6*7)'",
    )
    assert result.get("exit_code") == 0, result
    assert result["stdout_text"].split() == ["42", "42", "42"], result


def test_ollama_serves_on_the_workload_loopback(service, candidate, session):
    requires(candidate, "ollama")
    result = workload_exec(
        service.client(),
        session,
        "(ollama serve >/tmp/ollama.log 2>&1 &); "
        "for _ in $(seq 1 60); do curl -fsS http://127.0.0.1:11434/api/version && exit 0; sleep 1; done; "
        "cat /tmp/ollama.log; exit 1",
        timeout=90,
    )
    assert result.get("exit_code") == 0, result
    assert '"version"' in result["stdout_text"], result
