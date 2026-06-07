"""File I/O workflow tests."""

import json
import os

import pytest

from conftest import run


def test_file_write_read(output_dir):
    """Write text to a file and read it back."""
    test_file = output_dir / "write_read_test.txt"
    payload = "capsem file write test"
    test_file.write_text(payload)
    assert test_file.read_text() == payload


def test_python_json_roundtrip(output_dir):
    """Python json.dump -> json.load roundtrip."""
    test_file = output_dir / "json_roundtrip.json"
    data = {"key": "value", "nums": [1, 2, 3], "nested": {"ok": True}}
    with open(test_file, "w") as f:
        json.dump(data, f)
    with open(test_file) as f:
        loaded = json.load(f)
    assert loaded == data


def test_node_file_roundtrip(output_dir):
    """Node writes a file, Python reads it back."""
    test_file = output_dir / "node_roundtrip.json"
    js_code = (
        'const fs = require("fs"); '
        f'fs.writeFileSync("{test_file}", '
        'JSON.stringify({from: "node", ok: true}));'
    )
    result = run(f"node -e '{js_code}'")
    assert result.returncode == 0, f"node failed: {result.stderr}"
    data = json.loads(test_file.read_text())
    assert data["ok"] is True
    assert data["from"] == "node"


def test_pipe_workflow(output_dir):
    """Shell pipe chain writes expected output."""
    test_file = output_dir / "pipe_test.txt"
    result = run(
        f'echo "Hello Capsem World" | grep -o "Capsem" | tr "[:lower:]" "[:upper:]" > {test_file}'
    )
    assert result.returncode == 0
    content = test_file.read_text().strip()
    assert content == "CAPSEM"


def test_large_file_write(output_dir):
    """Write a 10MB file to tmpfs and verify size."""
    test_file = output_dir / "large_test.bin"
    result = run(f"dd if=/dev/zero of={test_file} bs=1M count=10 2>&1")
    assert result.returncode == 0, f"dd failed: {result.stderr}"
    size = os.path.getsize(test_file)
    assert size == 10 * 1024 * 1024, f"expected 10MB, got {size}"
    os.unlink(test_file)
