"""The workload's /home is the existing observed, host-visible file tree."""
import os
import runpy
from pathlib import Path

from tests.fixtures.oci.test_launch_config import SECURITY, image, launcher, unpacked

__all__ = ["launcher"]
SOURCE = Path(__file__).resolve().parents[3] / "guest/artifacts/container/runtime_home.py"


def test_home_mount_reuses_observed_share_and_masks_launcher(launcher):
    metadata = image()
    metadata["config"]["Volumes"] = {"/home/capsem": {}}
    config = launcher.configure(unpacked(), metadata, {**SECURITY, "args": [], "env": {}, "workspace": "/workspace"})
    mounts = config["mounts"]
    home = next(m for m in mounts if m["destination"] == "/home")
    assert home["source"] == str(launcher.WORKSPACE_VIEW)
    assert not any(m["destination"] == "/home/capsem" for m in mounts)
    mask = next(m for m in mounts if m["destination"] == "/home/.capsem-image")
    assert "ro" in mask["options"] and "mode=000" in mask["options"]


def test_home_seed_preserves_artifacts_reference_files_links_and_executables(tmp_path):
    seed = tmp_path / "seed"
    (seed / "capsem").mkdir(parents=True)
    (seed / "capsem/ascii_art.txt").write_bytes(b"demo artifact")
    (seed / "capsem/credential-ref.json").write_bytes(b'{"ref":"synthetic-hash"}')
    tool = seed / "capsem/tool"
    tool.write_bytes(b"#!/bin/sh\n")
    tool.chmod(0o755)
    (seed / "capsem/link").symlink_to("tool")
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    receipts = tmp_path / "receipts"
    receipts.mkdir()
    module = runpy.run_path(str(SOURCE))
    module["prepare"]([((), seed)], workspace, receipts, None, None)
    assert (workspace / "capsem/ascii_art.txt").read_bytes() == b"demo artifact"
    assert (workspace / "capsem/credential-ref.json").read_bytes() == b'{"ref":"synthetic-hash"}'
    assert os.readlink(workspace / "capsem/link") == "tool"
    assert (workspace / "capsem/tool").stat().st_mode & 0o777 == 0o755
    (workspace / "capsem/ascii_art.txt").write_bytes(b"newer data")
    module["prepare"]([((), seed)], workspace, receipts, None, None)
    assert (workspace / "capsem/ascii_art.txt").read_bytes() == b"newer data"
    assert (seed / "capsem/ascii_art.txt").read_bytes() == b"demo artifact"
