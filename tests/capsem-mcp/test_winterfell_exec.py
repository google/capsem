"""Winterfell persistence test: exec path (shell commands for write/read).

Same lifecycle as test_winterfell_rw but uses capsem_exec to write and read
files via shell commands instead of the write_file/read_file API.
"""

import json
import time
import uuid

import pytest

pytestmark = pytest.mark.mcp


def parse_content(result):
    return json.loads(result["content"][0]["text"])


def content_text(result):
    return result["content"][0]["text"]


def wait_ready(session, vm_name, timeout=30):
    for _ in range(timeout):
        try:
            res = session.call_tool("capsem_exec", {
                "id": vm_name, "command": "echo ready",
            })
            if "ready" in content_text(res):
                return True
        except (AssertionError, KeyError):
            pass
        time.sleep(1)
    return False


def test_winterfell_exec(mcp_session):
    name = f"wf-ex-{uuid.uuid4().hex[:4]}"
    message = "winter is coming"
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

        # 4. Write "winter is coming" via exec
        res = mcp_session.call_tool("capsem_exec", {
            "id": name,
            "command": f"echo '{message}' > {path}",
        })
        data = parse_content(res)
        assert data["exit_code"] == 0

        # 5. Read it back via exec
        res = mcp_session.call_tool("capsem_exec", {
            "id": name, "command": f"cat {path}",
        })
        data = parse_content(res)
        assert data["exit_code"] == 0
        assert message in data["stdout"]

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
        assert wait_ready(mcp_session, name, timeout=30), (
            f"{name} not exec-ready after resume"
        )

        # 10. Read file via exec -- must survive stop/resume
        res = mcp_session.call_tool("capsem_exec", {
            "id": name, "command": f"cat {path}",
        })
        data = parse_content(res)
        assert data["exit_code"] == 0
        assert message in data["stdout"], (
            f"File did not survive stop+resume: stdout={data['stdout']}"
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
