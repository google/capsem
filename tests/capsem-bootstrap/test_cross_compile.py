"""Cross-compiled guest binary validation."""

import os
import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent

pytestmark = pytest.mark.bootstrap

GUEST_BINARIES = ["capsem-pty-agent", "capsem-net-proxy", "capsem-mcp-server"]


def _host_arch():
    return "arm64" if os.uname().machine == "arm64" else "x86_64"


def _agent_dir():
    arch = _host_arch()
    return PROJECT_ROOT / "target" / "linux-agent" / arch


class TestGuestBinaries:

    def test_binaries_exist(self):
        agent_dir = _agent_dir()
        if not agent_dir.exists():
            pytest.skip(f"Agent dir not found: {agent_dir}")
        for name in GUEST_BINARIES:
            binary = agent_dir / name
            assert binary.exists(), f"Guest binary not found: {binary}"

    def test_binaries_are_elf(self):
        agent_dir = _agent_dir()
        if not agent_dir.exists():
            pytest.skip(f"Agent dir not found: {agent_dir}")
        for name in GUEST_BINARIES:
            binary = agent_dir / name
            if not binary.exists():
                continue
            result = subprocess.run(["file", str(binary)], capture_output=True, text=True)
            assert "ELF" in result.stdout, f"{name} is not an ELF binary: {result.stdout}"

    def test_binaries_correct_arch(self):
        agent_dir = _agent_dir()
        if not agent_dir.exists():
            pytest.skip(f"Agent dir not found: {agent_dir}")
        arch = _host_arch()
        expected_arch = "aarch64" if arch == "arm64" else "x86-64"
        for name in GUEST_BINARIES:
            binary = agent_dir / name
            if not binary.exists():
                continue
            result = subprocess.run(["file", str(binary)], capture_output=True, text=True)
            assert expected_arch in result.stdout, (
                f"{name} has wrong arch. Expected {expected_arch}, got: {result.stdout}"
            )

    def test_binaries_statically_linked(self):
        agent_dir = _agent_dir()
        if not agent_dir.exists():
            pytest.skip(f"Agent dir not found: {agent_dir}")
        for name in GUEST_BINARIES:
            binary = agent_dir / name
            if not binary.exists():
                continue
            result = subprocess.run(["file", str(binary)], capture_output=True, text=True)
            assert "statically linked" in result.stdout, (
                f"{name} should be statically linked (musl): {result.stdout}"
            )

    def test_binaries_executable(self):
        agent_dir = _agent_dir()
        if not agent_dir.exists():
            pytest.skip(f"Agent dir not found: {agent_dir}")
        for name in GUEST_BINARIES:
            binary = agent_dir / name
            if not binary.exists():
                continue
            assert os.access(binary, os.X_OK), f"{name} is not executable"
