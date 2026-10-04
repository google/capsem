"""The launcher's `--exec`: a command run inside the running workload.

`POST /vms/{id}/exec` on an image session reaches here (capsem-core
`workload_exec_command`). It must run as the workload's own process -- the
image's user, cwd and env, with the hardening `configure` gave it -- and
refuse plainly when there is no workload to enter, never fall back to the VM.
"""

import json
import os
import subprocess

import pytest

from tests.fixtures.oci.test_launch_config import SECURITY, image, launcher, unpacked

__all__ = ["launcher"]


def encoded(request):
    return json.dumps(request).encode().hex()


def workload_config(launcher):
    config = launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {"APP": "1"}})
    config["process"]["user"] = {"uid": 999, "gid": 999, "additionalGids": [999, 1000]}
    return config


def test_a_command_runs_under_the_image_shell(launcher):
    hostile = "printf '%s' \"$(id -u)\"; echo 'a'\\''b'\n"
    assert launcher.exec_request(encoded({"command": hostile, "tty": False})) == (["/bin/sh", "-c", hostile], False)
    assert launcher.exec_request(encoded({"argv": ["/bin/sh", "-l"], "tty": True})) == (["/bin/sh", "-l"], True)
    assert launcher.exec_request(encoded({"argv": ["id"]})) == (["id"], False)


@pytest.mark.parametrize(
    "request_value",
    [
        {},
        {"command": "id", "argv": ["id"]},
        {"command": "id", "user": 0},
        {"command": "id", "tty": 1},
        {"argv": []},
        {"argv": [""]},
        {"argv": ["id", 1]},
        {"argv": ["id", "a\0b"]},
        {"argv": "id"},
        ["id"],
    ],
)
def test_a_malformed_request_is_refused(launcher, request_value):
    with pytest.raises(ValueError):
        launcher.exec_request(encoded(request_value))


def test_a_request_that_is_not_hex_json_is_refused(launcher):
    for value in ("not hex!", b"{not json".hex(), "7b2"):
        with pytest.raises(ValueError):
            launcher.exec_request(value)


def test_the_exec_is_the_workload_process_with_another_command(launcher):
    config = workload_config(launcher)
    before = json.dumps(config)
    process = launcher.exec_process(config, ["id", "-u"], True)
    workload = config["process"]
    assert process["args"] == ["id", "-u"]
    assert process["terminal"] is True
    # The image's user (with its groups), working directory and environment.
    assert process["user"] == {"uid": 999, "gid": 999, "additionalGids": [999, 1000]}
    assert process["cwd"] == "/data"
    assert process["env"] == workload["env"]
    assert "APP=1" in process["env"]
    assert any(entry.startswith("SSL_CERT_FILE=") for entry in process["env"])
    # The hardening the workload runs under, not runc's exec defaults.
    assert process["noNewPrivileges"] is True
    assert process["capabilities"] == workload["capabilities"]
    assert process["rlimits"] == workload["rlimits"]
    assert json.dumps(config) == before, "the bundle's own config is never changed"


def test_runc_execs_into_the_workload_through_the_launch_state_root(launcher):
    assert launcher.exec_argv("/proc/self/fd/3") == [
        "runc",
        "--rootless=true",
        "--root",
        str(launcher.RUNTIME / "state"),
        "exec",
        "--process",
        "/proc/self/fd/3",
        launcher.CONTAINER,
    ]
    # Exactly where the launch put the container, so exec finds that one.
    assert launcher.RUNC[-1] == str(launcher.RUNTIME / "state")


def _runc_state(status, bundle="/var/lib/capsem/roots/ab/bundle", returncode=0):
    def run(*args, **kwargs):
        assert args[-2:] == ("state", "workload"), args
        return subprocess.CompletedProcess(args, returncode, stdout=json.dumps({"status": status, "bundle": bundle}))

    return run


def test_only_a_running_workload_can_be_entered_through_the_bundle_runc_names(launcher, tmp_path, monkeypatch):
    monkeypatch.setattr(launcher, "RUNTIME", tmp_path)

    def never(*args, **kwargs):
        raise AssertionError("runc asked about a container that was never created")

    assert launcher.workload_bundle(run=never) is None
    (tmp_path / "state" / "workload").mkdir(parents=True)
    # The bundle lives under the digest's unpacked root, not under RUNTIME.
    assert launcher.workload_bundle(run=_runc_state("running")) == launcher.Path("/var/lib/capsem/roots/ab/bundle")
    assert launcher.workload_bundle(run=_runc_state("stopped")) is None
    assert launcher.workload_bundle(run=_runc_state("running", returncode=1)) is None


def test_exec_without_a_workload_is_refused_not_run_in_the_vm(launcher, tmp_path, monkeypatch):
    monkeypatch.setattr(launcher, "RUNTIME", tmp_path)

    def must_not_exec(*args):
        raise AssertionError(f"exec'd {args} with no workload")

    monkeypatch.setattr(launcher.os, "execvp", must_not_exec)
    with pytest.raises(SystemExit) as refused:
        launcher.exec_workload(encoded({"command": "id -u"}))
    assert "no container workload is running" in str(refused.value)


def test_exec_hands_runc_the_workload_process_and_nothing_else(launcher, tmp_path, monkeypatch):
    monkeypatch.setattr(launcher, "RUNTIME", tmp_path / "runtime")
    bundle = tmp_path / "roots" / "ab" / "bundle"
    bundle.mkdir(parents=True)
    config = workload_config(launcher)
    (bundle / "config.json").write_text(json.dumps(config))
    monkeypatch.setattr(launcher, "workload_bundle", lambda: bundle)
    # memfd is Linux's; a plain file descriptor stands in for it on any host.
    monkeypatch.setattr(
        launcher.os,
        "memfd_create",
        lambda name, flags: os.open(tmp_path / name, os.O_RDWR | os.O_CREAT | os.O_EXCL, 0o600),
        raising=False,
    )
    seen = {}

    def execvp(program, argv):
        spec = argv[argv.index("--process") + 1]
        assert spec.startswith("/proc/self/fd/"), spec
        descriptor = int(spec.rsplit("/", 1)[1])
        assert os.get_inheritable(descriptor), "runc must inherit the process spec"
        os.lseek(descriptor, 0, os.SEEK_SET)
        seen.update(program=program, argv=argv, process=json.loads(os.read(descriptor, 1 << 20)))
        raise SystemExit(0)

    monkeypatch.setattr(launcher.os, "execvp", execvp)
    with pytest.raises(SystemExit):
        launcher.exec_workload(encoded({"command": "id -u"}))
    assert seen["program"] == "runc"
    assert seen["argv"][-1] == launcher.CONTAINER
    assert seen["process"] == launcher.exec_process(config, ["/bin/sh", "-c", "id -u"], False)
