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
    # A terminal gets a terminal type when the image names none.
    assert process["env"] == [*workload["env"], "TERM=xterm-256color"]
    assert launcher.exec_process(config, ["id"], False)["env"] == workload["env"]
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


def test_an_image_terminal_type_is_kept(launcher):
    config = workload_config(launcher)
    config["process"]["env"].append("TERM=dumb")
    env = launcher.exec_process(config, ["sh"], True)["env"]
    assert [entry for entry in env if entry.startswith("TERM=")] == ["TERM=dumb"]


class Stop(Exception):
    pass


def attach_trace(launcher, tmp_path, monkeypatch, states, sleeps):
    """Run `attach` over a scripted workload state until it has slept `sleeps`
    times; return what it ran and printed."""
    bundle = tmp_path / "bundle"
    bundle.mkdir()
    (bundle / "config.json").write_text(json.dumps(workload_config(launcher)))
    monkeypatch.setattr(
        launcher.os,
        "memfd_create",
        lambda name, flags: os.open(tmp_path / f"{name}-{len(ran)}", os.O_RDWR | os.O_CREAT | os.O_EXCL, 0o600),
        raising=False,
    )
    monkeypatch.setattr(launcher.signal, "signal", lambda number, handler: ignored.append((number, handler)))
    states, ran, ignored, slept = iter(states), [], [], []

    def run(argv, pass_fds, check):
        descriptor = int(argv[argv.index("--process") + 1].rsplit("/", 1)[1])
        assert pass_fds == (descriptor,) and check is False
        os.lseek(descriptor, 0, os.SEEK_SET)
        ran.append(json.loads(os.read(descriptor, 1 << 20)))

    def sleep(seconds):
        slept.append(seconds)
        if len(slept) == sleeps:
            raise Stop

    with pytest.raises(Stop):
        launcher.attach(bundle_of=lambda: bundle if next(states) else None, run=run, sleep=sleep)
    return ran, ignored


def test_the_terminal_waits_for_the_workload_then_enters_it_with_a_login_shell(
    launcher, tmp_path, monkeypatch, capsys
):
    ran, ignored = attach_trace(launcher, tmp_path, monkeypatch, [False, False, True], 3)
    assert len(ran) == 1
    assert ran[0]["args"] == launcher.TERMINAL_SHELL
    assert ran[0]["terminal"] is True
    assert ran[0]["user"]["uid"] == 999, "the terminal is the image's user, not the VM's root"
    assert "TERM=xterm-256color" in ran[0]["env"]
    assert capsys.readouterr().out.count("waiting for the workload") == 1
    signals = {number for number, handler in ignored if handler == launcher.signal.SIG_IGN}
    assert {launcher.signal.SIGINT, launcher.signal.SIGQUIT, launcher.signal.SIGTSTP} <= signals


def test_the_terminal_enters_the_workload_again_when_its_shell_ends(launcher, tmp_path, monkeypatch, capsys):
    ran, _ = attach_trace(launcher, tmp_path, monkeypatch, [True, True, False, True], 4)
    assert len(ran) == 3, "each ended shell is followed by a new one, never by a VM shell"
    assert all(process["args"] == launcher.TERMINAL_SHELL for process in ran)
    assert capsys.readouterr().out.count("entering it again") == 3

