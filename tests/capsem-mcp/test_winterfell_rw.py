"""Winterfell persistence test: write_file / read_file path.

The full "the north remembers" lifecycle via MCP tools:
  create -> list -> write_file -> read_file -> stop ->
  list (stopped) -> resume -> read_file (survives) ->
  delete -> list (gone) -> resume (fails)
"""

import uuid

import pytest

from helpers.constants import EXEC_READY_TIMEOUT
from helpers.mcp import content_text, parse_content
from helpers.mcp import wait_exec_ready as wait_ready

pytestmark = pytest.mark.mcp


def test_winterfell_rw(mcp_session):
    name = f"wf-rw-{uuid.uuid4().hex[:4]}"
    message = "the north remembers"
    path = "/root/stark_words.txt"

    # 1. Create persistent VM
    res = mcp_session.call_tool("capsem_create", {"name": name})
    assert name in content_text(res)

    try:
        # 2. List: running, persistent
        res = mcp_session.call_tool("capsem_list")
        listing = parse_content(res)
        vm = next((s for s in listing["sandboxes"] if s["id"] == name), None)
        assert vm is not None, f"{name} not in list"
        assert vm["status"] == "Running"
        assert vm["persistent"] is True

        # 3. Wait for exec-ready
        assert wait_ready(mcp_session, name), f"{name} never exec-ready"

        # 4. Write "the north remembers"
        res = mcp_session.call_tool("capsem_write_file", {
            "id": name, "path": path, "content": message,
        })
        data = parse_content(res)
        assert data.get("success") is True

        # 5. Read it back
        res = mcp_session.call_tool("capsem_read_file", {"id": name, "path": path})
        data = parse_content(res)
        assert data["content"] == message

        # 6. Stop (persistent preserves state)
        res = mcp_session.call_tool("capsem_stop", {"id": name})
        data = parse_content(res)
        assert data.get("success") is True

        # 7. List: stopped, pid=0, persistent
        res = mcp_session.call_tool("capsem_list")
        listing = parse_content(res)
        vm = next((s for s in listing["sandboxes"] if s["id"] == name), None)
        assert vm is not None, f"{name} vanished after stop"
        assert vm["status"] == "Stopped"
        assert vm["pid"] == 0
        assert vm["persistent"] is True

        # 8. Resume
        res = mcp_session.call_tool("capsem_resume", {"name": name})
        assert name in content_text(res)

        # 9. Wait for resumed VM to be exec-ready
        assert wait_ready(mcp_session, name, timeout=EXEC_READY_TIMEOUT), (
            f"{name} not exec-ready after resume"
        )

        # 10. Read file -- must survive stop/resume
        res = mcp_session.call_tool("capsem_read_file", {"id": name, "path": path})
        data = parse_content(res)
        assert data["content"] == message, (
            f"File did not survive stop+resume: {data}"
        )

        # 11. Delete
        mcp_session.call_tool("capsem_delete", {"id": name})

        # 12. List: gone
        res = mcp_session.call_tool("capsem_list")
        listing = parse_content(res)
        ids = [s["id"] for s in listing["sandboxes"]]
        assert name not in ids, f"{name} still in list after delete"

        # 13. Resume after delete: must fail
        resp = mcp_session.call_tool_raw("capsem_resume", {"name": name})
        result = resp.get("result", {})
        assert result.get("isError") is True or "error" in resp, (
            f"Resume after delete should fail, got: {resp}"
        )

    except Exception:
        try:
            mcp_session.call_tool("capsem_delete", {"id": name})
        except Exception:
            pass
        raise
