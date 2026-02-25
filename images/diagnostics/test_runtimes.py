"""Dev runtime version checks and execution tests."""

import json

import pytest

from conftest import run


@pytest.mark.parametrize("runtime", ["python3", "node", "npm", "pip3", "uv", "git"])
def test_runtime_version(runtime):
    """Each dev runtime must respond to --version."""
    result = run(f"{runtime} --version")
    assert result.returncode == 0, f"{runtime} --version failed: {result.stderr}"


def test_python_execution(output_dir):
    """Python can import stdlib, write JSON, and read it back."""
    out_file = output_dir / "python_exec_test.json"
    code = f"""
import json, os, math
data = {{"pi": math.pi, "pid": os.getpid(), "ok": True}}
with open("{out_file}", "w") as f:
    json.dump(data, f)
"""
    result = run(f'python3 -c \'{code}\'')
    assert result.returncode == 0, f"python3 failed: {result.stderr}"
    assert out_file.exists(), f"{out_file} not created"
    data = json.loads(out_file.read_text())
    assert data["ok"] is True
    assert abs(data["pi"] - 3.14159265) < 0.001


def test_node_execution(output_dir):
    """Node.js can write a JSON file via fs module."""
    out_file = output_dir / "node_exec_test.json"
    js_code = (
        'const fs = require("fs"); '
        f'fs.writeFileSync("{out_file}", '
        'JSON.stringify({node: true, v: process.version}));'
    )
    result = run(f"node -e '{js_code}'")
    assert result.returncode == 0, f"node failed: {result.stderr}"
    assert out_file.exists(), f"{out_file} not created"
    data = json.loads(out_file.read_text())
    assert data["node"] is True


def test_git_workflow(output_dir):
    """Git can init, configure, commit, and show log."""
    repo = output_dir / "git_test_repo"
    cmds = " && ".join([
        f"rm -rf {repo}",
        f"mkdir -p {repo}",
        f"cd {repo}",
        "git init",
        "git config user.email test@capsem.local",
        "git config user.name capsem-test",
        "echo hello > readme.txt",
        "git add readme.txt",
        'git commit -m "test commit"',
        "git log --oneline",
    ])
    result = run(cmds, timeout=15)
    assert result.returncode == 0, f"git workflow failed: {result.stderr}"
    assert "test commit" in result.stdout
