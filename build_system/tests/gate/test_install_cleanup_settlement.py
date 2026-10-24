"""A failed external owner removal cannot masquerade as settled resources."""

from pathlib import Path
from subprocess import CompletedProcess

import pytest
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.installcontainer import InstallContainer
from helpers.gate import RecordingRunner


class RefusedRemoval(RecordingRunner):
    def execute(self, command):
        if command.argv[:3] == ("docker", "rm", "-f"):
            return CompletedProcess(
                list(command.argv), 1, stdout="", stderr="synthetic daemon refusal"
            )
        return super().execute(command)


def test_final_installed_container_removal_must_succeed():
    root = Path(__file__).resolve().parents[3]
    container = InstallContainer(RefusedRemoval(root))
    with pytest.raises(GateError, match="command failed"):
        container.stop()
