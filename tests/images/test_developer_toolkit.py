"""Pinned shared guest toolkit and safe archive materialization."""
from __future__ import annotations

import importlib.util
import io
import json
import re
import tarfile
import zipfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("developer_toolkit", ROOT / "images/base/install-developer-tools.py")
assert SPEC and SPEC.loader
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)


def test_requested_tools_are_unique_and_release_artifacts_are_pinned_for_both_architectures() -> None:
    manifest = json.loads((ROOT / "images/base/developer-tools.json").read_text())
    names = [tool["name"] for tool in manifest["tools"]]
    assert len(names) == len(set(names))
    assert set(names) == {"bash", "zsh", "tmux", "fzf", "starship", "git", "git-lfs", "gh", "delta", "lazygit",
                          "ripgrep", "fd", "jq", "yq", "bat", "ast-grep", "ugrep", "curl", "wget"}
    for tool in manifest["downloads"]:
        assert set(tool["artifacts"]) == {"arm64", "amd64"}
        for artifact in tool["artifacts"].values():
            assert re.fullmatch(r"[a-f0-9]{64}", artifact["sha256"])
            assert artifact["url"].startswith("https://github.com/")
            assert tool["version"] in artifact["url"]
    yq = next(tool for tool in manifest["downloads"] if tool["name"] == "yq")
    assert all("mikefarah/yq/" in artifact["url"] for artifact in yq["artifacts"].values())


def test_tar_binary_is_read_without_extracting_traversal_paths_or_links(tmp_path: Path) -> None:
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode="w:gz") as package:
        member = tarfile.TarInfo("../../outside/tool")
        member.size = 4
        package.addfile(member, io.BytesIO(b"body"))
        link = tarfile.TarInfo("tool-link")
        link.type = tarfile.SYMTYPE
        link.linkname = "/etc/passwd"
        package.addfile(link)
    assert installer.binary_bytes(data.getvalue(), "tar", "tool") == b"body"
    with pytest.raises(ValueError, match="regular archive binary"):
        installer.binary_bytes(data.getvalue(), "tar", "tool-link")
    assert list(tmp_path.iterdir()) == []


def test_zip_refuses_an_ambiguous_binary() -> None:
    data = io.BytesIO()
    with zipfile.ZipFile(data, "w") as package:
        package.writestr("a/tool", b"a")
        package.writestr("b/tool", b"b")
    with pytest.raises(ValueError, match="one archive binary"):
        installer.binary_bytes(data.getvalue(), "zip", "tool")
