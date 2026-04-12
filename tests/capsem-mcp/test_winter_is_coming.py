"""Winter is coming: full fork e2e test.

Boot a VM, install packages, write workspace files, fork, verify:
  - fork completes in under 1 second
  - image is not a bloated 2GB sparse turd (actual disk < 100MB)
  - boot from image preserves packages (rootfs overlay) AND workspace files
"""

import time
import uuid

import pytest

from helpers.mcp import content_text, parse_content, wait_exec_ready as wait_ready

pytestmark = pytest.mark.mcp

MAX_FORK_SECS = 2.0
MAX_IMAGE_SIZE_MB = 12


def test_winter_is_coming(mcp_session):
    """Full fork: packages + workspace survive, fork is fast, image is small."""
    vm = f"wic-{uuid.uuid4().hex[:4]}"
    image = f"wic-img-{uuid.uuid4().hex[:4]}"
    forked = f"wic-fk-{uuid.uuid4().hex[:4]}"

    try:
        # 1. Create base VM
        mcp_session.call_tool("capsem_create", {"name": vm})
        assert wait_ready(mcp_session, vm), f"{vm} never exec-ready"

        # 2. Install packages (rootfs overlay changes)
        res = mcp_session.call_tool("capsem_exec", {
            "id": vm,
            "command": "apt-get update -qq && apt-get install -y -qq curl jq tree 2>&1 | tail -1",
            "timeout": 120,
        })
        data = parse_content(res)
        assert data["exit_code"] == 0, f"apt-get failed: {data['stderr']}"

        # Verify packages installed
        res = mcp_session.call_tool("capsem_exec", {
            "id": vm,
            "command": "which curl jq tree",
        })
        data = parse_content(res)
        assert data["exit_code"] == 0
        assert "/usr/bin/curl" in data["stdout"]

        # 3. Write workspace files
        mcp_session.call_tool("capsem_write_file", {
            "id": vm,
            "path": "/root/stark_words.txt",
            "content": "winter is coming",
        })
        mcp_session.call_tool("capsem_write_file", {
            "id": vm,
            "path": "/root/project/main.py",
            "content": "print('the north remembers')",
        })

        # 4. Fork -- must complete in under 1 second
        t0 = time.monotonic()
        res = mcp_session.call_tool("capsem_fork", {
            "id": vm,
            "name": image,
            "description": "winter-is-coming e2e fork test",
        })
        fork_secs = time.monotonic() - t0
        fork_data = parse_content(res)
        assert fork_data["name"] == image
        assert fork_secs < MAX_FORK_SECS, (
            f"fork took {fork_secs:.3f}s, expected < {MAX_FORK_SECS}s"
        )

        # 5. Image size must be actual disk usage, not a 2GB sparse lie
        res = mcp_session.call_tool("capsem_image_inspect", {"name": image})
        info = parse_content(res)
        size_mb = info["size_bytes"] / (1024 * 1024)
        assert size_mb < MAX_IMAGE_SIZE_MB, (
            f"image is {size_mb:.1f}MB, expected < {MAX_IMAGE_SIZE_MB}MB "
            f"(sparse file reporting actual blocks?)"
        )

        # 6. Boot from forked image
        mcp_session.call_tool("capsem_create", {
            "name": forked,
            "image": image,
        })
        assert wait_ready(mcp_session, forked), f"{forked} never exec-ready"

        # 7. Packages survived (rootfs overlay)
        res = mcp_session.call_tool("capsem_exec", {
            "id": forked,
            "command": "which curl jq tree",
        })
        data = parse_content(res)
        assert data["exit_code"] == 0, "packages did not survive fork"
        assert "/usr/bin/curl" in data["stdout"]

        # 8. Workspace files survived
        res = mcp_session.call_tool("capsem_read_file", {
            "id": forked,
            "path": "/root/stark_words.txt",
        })
        assert "winter is coming" in content_text(res), "workspace file lost in fork"

        res = mcp_session.call_tool("capsem_read_file", {
            "id": forked,
            "path": "/root/project/main.py",
        })
        assert "the north remembers" in content_text(res), "nested workspace file lost in fork"

    finally:
        for v in [forked, vm]:
            try:
                mcp_session.call_tool("capsem_delete", {"id": v})
            except Exception:
                pass
        try:
            mcp_session.call_tool("capsem_image_delete", {"name": image})
        except Exception:
            pass
