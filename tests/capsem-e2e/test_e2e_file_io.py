"""E2E file I/O tests via real CLI exec commands.

Write files inside the VM via exec, read them back, verify content.
No UdsHttpClient -- just capsem exec with shell commands.
"""

import uuid

import pytest

pytestmark = pytest.mark.e2e


def _name(prefix="fio"):
    return f"{prefix}-{uuid.uuid4().hex[:8]}"


@pytest.fixture(scope="module")
def vm(service):
    """A single exec-ready VM for all file I/O tests."""
    name = _name()
    service.cli_ok("start", "--rm", "--name", name)
    assert service.wait_exec_ready(name), f"VM {name} never exec-ready"
    yield name
    service.cli("delete", name)


class TestFileIO:

    def test_write_and_read(self, service, vm):
        service.cli_ok("exec", vm, "echo payload-xyz > /tmp/test.txt")
        r = service.cli_ok("exec", vm, "cat /tmp/test.txt")
        assert "payload-xyz" in r.stdout

    def test_multiline(self, service, vm):
        service.cli_ok("exec", vm,
                        "printf 'line1\\nline2\\nline3\\n' > /tmp/multi.txt")
        r = service.cli_ok("exec", vm, "cat /tmp/multi.txt")
        assert "line1" in r.stdout
        assert "line2" in r.stdout
        assert "line3" in r.stdout

    def test_unicode(self, service, vm):
        service.cli_ok("exec", vm,
                        "echo 'cafe unicode test' > /tmp/uni.txt")
        r = service.cli_ok("exec", vm, "cat /tmp/uni.txt")
        assert "cafe unicode test" in r.stdout

    def test_overwrite(self, service, vm):
        service.cli_ok("exec", vm, "echo first > /tmp/ow.txt")
        service.cli_ok("exec", vm, "echo second > /tmp/ow.txt")
        r = service.cli_ok("exec", vm, "cat /tmp/ow.txt")
        assert "second" in r.stdout
        assert "first" not in r.stdout

    def test_nested_directory(self, service, vm):
        service.cli_ok("exec", vm,
                        "mkdir -p /tmp/deep/nested && "
                        "echo deep-content > /tmp/deep/nested/f.txt")
        r = service.cli_ok("exec", vm, "cat /tmp/deep/nested/f.txt")
        assert "deep-content" in r.stdout

    def test_read_nonexistent(self, service, vm):
        r = service.cli("exec", vm, "cat /tmp/no-such-file-xyz.txt")
        assert r.returncode != 0 or "No such file" in r.stdout + r.stderr

    def test_binary_roundtrip(self, service, vm):
        """Write and read back a known binary pattern via base64."""
        service.cli_ok("exec", vm,
                        "echo AQIDBA== | base64 -d > /tmp/bin.dat")
        r = service.cli_ok("exec", vm,
                           "base64 /tmp/bin.dat")
        assert "AQIDBA==" in r.stdout
