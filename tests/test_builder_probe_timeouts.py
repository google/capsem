"""Fast adversarial contracts for image-builder subprocess ownership."""

from __future__ import annotations

import subprocess
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from capsem.builder.docker import (
    CONTAINER_PROBE_CLEANUP_TIMEOUT_SECONDS,
    CONTAINER_PROBE_TIMEOUT_SECONDS,
    OBOM_COMMAND_TIMEOUT_SECONDS,
    _container_output,
    generate_cyclonedx_obom,
)


@patch("capsem.builder.docker.run_cmd")
def test_container_probe_timeout_force_removes_named_container(mock_run) -> None:
    mock_run.side_effect = [
        subprocess.TimeoutExpired(["docker", "run"], CONTAINER_PROBE_TIMEOUT_SECONDS),
        MagicMock(stdout=""),
    ]

    with pytest.raises(RuntimeError, match=r"dpkg inventory.*timed out"):
        _container_output(
            "docker",
            "capsem-rootfs-arm64",
            "linux/arm64",
            "dpkg-query -W",
            probe="dpkg inventory",
        )

    run_call, cleanup_call = mock_run.call_args_list
    run_command = run_call.args[0]
    assert run_command[:3] == ["docker", "run", "--rm"]
    assert run_command[run_command.index("--pull") + 1] == "never"
    assert run_command[run_command.index("--network") + 1] == "none"
    name = run_command[run_command.index("--name") + 1]
    assert name.startswith("capsem-probe-dpkg-inventory-")
    assert run_call.kwargs["timeout"] == CONTAINER_PROBE_TIMEOUT_SECONDS

    assert cleanup_call.args[0] == ["docker", "rm", "-f", name]
    assert cleanup_call.kwargs == {
        "capture": True,
        "echo": False,
        "timeout": CONTAINER_PROBE_CLEANUP_TIMEOUT_SECONDS,
    }


@patch("capsem.builder.docker.run_cmd")
def test_successful_container_probe_is_bounded_without_cleanup(mock_run) -> None:
    mock_run.return_value = MagicMock(stdout="inventory\n")

    output = _container_output(
        "docker",
        "capsem-rootfs-x86_64",
        "linux/amd64",
        "dpkg-query -W",
        probe="dpkg inventory",
    )

    assert output == "inventory\n"
    assert mock_run.call_count == 1
    assert mock_run.call_args.kwargs["timeout"] == CONTAINER_PROBE_TIMEOUT_SECONDS


@patch("capsem.builder.docker._validate_cyclonedx_obom")
@patch("capsem.builder.docker._normalize_cyclonedx_obom")
@patch("capsem.builder.docker.run_cmd")
def test_obom_subprocesses_are_all_bounded(
    mock_run,
    _mock_normalize,
    _mock_validate,
    tmp_path: Path,
) -> None:
    rootfs_tar = tmp_path / "rootfs.tar"
    rootfs_tar.write_bytes(b"rootfs")
    output = tmp_path / "obom.cdx.json"

    generate_cyclonedx_obom(
        rootfs_tar,
        output,
        repo_root=tmp_path,
        architecture="arm64",
        runtime="docker",
        tool_image="capsem-asset-tools-arm64:test",
        tool_platform="linux/arm64",
        runtime_network="none",
    )

    assert len(mock_run.call_args_list) == 3
    assert {call.kwargs.get("timeout") for call in mock_run.call_args_list} == {
        OBOM_COMMAND_TIMEOUT_SECONDS
    }
