"""Images and trusted launch controls reach the VM on a read-only share.

The writable stage holds runtime markers. Host-owned options, controllers
and verified image blobs stay on a device that refuses guest writes even
after a read-write remount. Forks link immutable blobs and copy controls;
guest-writable fake controls never replace the trusted inputs on restart.
"""

import hashlib
import json
import os
from pathlib import Path

import pytest
from helpers.service import vm_session_dir

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import created, service
from tests.ironbank.kingslanding.test_workload_exec import run

__all__ = ["service"]

pytestmark = pytest.mark.integration

#: Trusted control inventory, separate from image blobs and runtime markers.
CONTROL_FILES = {
    "options.json", "launch.py", "runtime_limits.py", "runtime_preparation.py", "runtime_console.py", "runtime_pty.py",
    "runtime_terminal.py", "runtime_userns.py", "runtime_workspace.py", "runtime_home.py",
}
MARKERS = {"ready", "running", "failed", "exited"}
MOUNT = "/run/image-share-probe"


def stage_of(session):
    return session / "guest" / "workspace" / ".capsem-image"


def share_of(session):
    return session / "image"


def image_blobs(share, manifest):
    """The manifest the stage names and every blob it references, as hex."""
    document = json.loads((share / manifest.removeprefix("sha256:")).read_bytes())
    digests = [manifest, document["config"]["digest"], *(layer["digest"] for layer in document["layers"])]
    return {digest.removeprefix("sha256:") for digest in digests}, document


def assert_stage_holds_no_image(session, layers):
    stage = stage_of(session)
    names = {path.name for path in stage.iterdir()}
    assert {"ready", "running"} <= names <= MARKERS, names
    for path in stage.iterdir():
        # The launcher's source is the largest control file, a few dozen KiB.
        assert path.stat().st_size < 64 * 1024, path
    sizes = {layer["size"] for layer in layers}
    # Regular files only: the guest leaves links in its workspace (a dangling
    # `.venv`, for one), and a link is no copy of a layer.
    for root, _, files in os.walk(session / "guest" / "workspace"):
        for name in files:
            path = Path(root) / name
            if path.is_symlink() or not path.is_file():
                continue
            if path.stat().st_size in sizes:
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                assert f"sha256:{digest}" not in {layer["digest"] for layer in layers}, path


def guest_probe(blobs, victim):
    """Mount the share as guest root, without `ro`, list and hash it, then try
    every kind of write, before and after a read-write remount. Prints one
    `WROTE:<kind>` line for any write that took effect."""
    target = f"{MOUNT}/{victim}"
    writes = " ".join(
        [
            f"touch {MOUNT}/planted 2>/dev/null && echo WROTE:create;",
            f"(printf x >> {target}) 2>/dev/null && echo WROTE:append;",
            f"truncate -s 0 {target} 2>/dev/null && echo WROTE:truncate;",
            f"chmod 666 {target} 2>/dev/null && echo WROTE:chmod;",
            f"mv {target} {MOUNT}/moved 2>/dev/null && echo WROTE:rename;",
            f"ln {target} {MOUNT}/hardlink 2>/dev/null && echo WROTE:link;",
            f"ln -s /etc/shadow {MOUNT}/symlink 2>/dev/null && echo WROTE:symlink;",
            f"mkdir {MOUNT}/directory 2>/dev/null && echo WROTE:mkdir;",
            f"mknod {MOUNT}/fifo p 2>/dev/null && echo WROTE:mknod;",
            f"rm -f {target} 2>/dev/null; test -e {target} || echo WROTE:unlink;",
        ]
    )
    return (
        f"set -u; mkdir -p {MOUNT}; "
        f"mount -t virtiofs capsem-image {MOUNT} || mount -t virtiofs -o ro capsem-image {MOUNT}; "
        f'echo "OPTIONS $(grep " {MOUNT} " /proc/mounts | cut -d" " -f4)"; '
        f'echo "LIST $(ls -A {MOUNT} | sort | tr "\\n" " ")"; '
        f'echo "HASHES $(cd {MOUNT} && sha256sum {" ".join(sorted(blobs))} | cut -d" " -f1 | tr "\\n" " ")"; '
        f"{writes} "
        f"mount -o remount,rw {MOUNT} 2>/dev/null && echo REMOUNTED; "
        f"{writes} "
        f'echo "AFTER $(ls -A {MOUNT} | sort | tr "\\n" " ")"; '
        f"umount {MOUNT}"
    )


def fields(output):
    return {line.split(" ", 1)[0]: line.split(" ", 1)[1].strip() for line in output.splitlines() if " " in line}


def test_the_image_reaches_the_guest_through_a_read_only_share_of_its_blobs(service, tmp_path):
    client = service.client()
    with (
        registry(tmp_path) as (reference, certificate, _),
        created(service, tmp_path, reference, certificate, "image-share") as vm,
    ):
        session = vm_session_dir(service.tmp_dir, client, vm["id"])
        options = json.loads((share_of(session) / "options.json").read_text())
        manifest = options["manifest"]
        share = share_of(session)
        expected, document = image_blobs(share, manifest)
        listed = {path.name for path in share.iterdir()}
        assert listed == expected | CONTROL_FILES, "the share holds verified blobs and trusted controls"
        for path in share.iterdir():
            assert path.is_file() and not path.is_symlink(), path
            if path.name in expected:
                assert hashlib.sha256(path.read_bytes()).hexdigest() == path.name, path
                assert path.stat().st_mode & 0o777 == 0o444, path
        assert_stage_holds_no_image(session, document["layers"])

        victim = document["layers"][0]["digest"].removeprefix("sha256:")
        originals = {name: (share / name).read_bytes() for name in CONTROL_FILES}
        for selected in [victim, "options.json", "launch.py"]:
            output = run(client, vm["id"], guest_probe(expected, selected), target="vm")
            (tmp_path / f"guest-probe-{selected}.txt").write_text(output)
            seen = fields(output)
            assert set(seen["LIST"].split()) == listed, output
            assert seen["HASHES"].split() == sorted(expected), "the guest reads the verified bytes"
            assert set(seen["AFTER"].split()) == listed, output
            assert "WROTE:" not in output, output
        assert run(client, vm["id"], "test ! -r /run/capsem-image/options.json && test ! -r /workspace/.capsem-image/options.json && echo private").strip() == "private"
        # The host's copy is untouched, whatever the guest tried.
        assert {path.name for path in share.iterdir()} == listed
        assert hashlib.sha256((share / victim).read_bytes()).hexdigest() == victim
        assert {name: (share / name).read_bytes() for name in CONTROL_FILES} == originals
        # Fake controls in the writable workspace cannot replace launch inputs
        # on the next generation or a fork.
        run(client, vm["id"], "printf 'raise SystemExit(99)\\n' > /root/.capsem-image/launch.py; printf '{}' > /root/.capsem-image/options.json", target="vm")

        # A fork links its source's blobs; nothing comes back from the guest.
        fork_id = client.post(f"/vms/{vm['id']}/fork", {"name": "image-share-fork"})["id"]
        try:
            fork = vm_session_dir(service.tmp_dir, client, fork_id)
            assert {path.name for path in share_of(fork).iterdir()} == listed
            for name in expected:
                assert (share_of(fork) / name).stat().st_ino == (share / name).stat().st_ino, name
            for name in CONTROL_FILES:
                assert (share_of(fork) / name).read_bytes() == originals[name]
                assert (share_of(fork) / name).stat().st_ino != (share / name).stat().st_ino
            assert "error" not in client.post(f"/vms/{fork_id}/resume", {}, timeout=120)
            assert run(client, fork_id, "printf TRUSTED_FORK").strip() == "TRUSTED_FORK"
            assert "error" not in client.post(f"/vms/{vm['id']}/stop", {}, timeout=60)
            assert "error" not in client.post(f"/vms/{vm['id']}/resume", {}, timeout=120)
            assert run(client, vm["id"], "printf TRUSTED_RESUME").strip() == "TRUSTED_RESUME"
        finally:
            client.delete(f"/vms/{fork_id}/delete")
