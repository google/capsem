"""An image reaches its VM through a read-only share holding only its blobs.

`capsem create --image` used to copy every layer of the image into the
workspace stage, which the guest can write. The stage now keeps two small
control files and not one layer byte; the image's blobs sit in the session's
host-only image share, each named by its SHA-256, which the VM owner attaches
as a read-only VirtioFS device. Read-only is the device's property: guest
root mounting it without `ro`, or remounting it read-write, still cannot
create, change, rename or remove anything in it. A fork links its source's
blobs instead of reading anything back from the guest.
"""

import hashlib
import json
import os

import pytest
from helpers.service import vm_session_dir

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import created, service
from tests.ironbank.kingslanding.test_workload_exec import run

__all__ = ["service"]

pytestmark = pytest.mark.integration

#: What the workspace stage may hold: the launcher's inputs and its markers.
CONTROL_FILES = {"options.json", "launch.py"}
MARKERS = {"ready", "running", "failed"}
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
    assert CONTROL_FILES <= names <= CONTROL_FILES | MARKERS, names
    for path in stage.iterdir():
        # The launcher's source is the largest control file, a few dozen KiB.
        assert path.stat().st_size < 64 * 1024, path
    sizes = {layer["size"] for layer in layers}
    for root, _, files in os.walk(session / "guest" / "workspace"):
        for name in files:
            path = os.path.join(root, name)
            if os.path.getsize(path) in sizes:
                digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
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
        options = json.loads((stage_of(session) / "options.json").read_text())
        manifest = options["manifest"]
        share = share_of(session)
        expected, document = image_blobs(share, manifest)
        listed = {path.name for path in share.iterdir()}
        assert listed == expected, "the share holds that image's blobs and nothing else"
        for path in share.iterdir():
            assert path.is_file() and not path.is_symlink(), path
            assert hashlib.sha256(path.read_bytes()).hexdigest() == path.name, path
            assert path.stat().st_mode & 0o777 == 0o444, path
        assert_stage_holds_no_image(session, document["layers"])

        victim = document["layers"][0]["digest"].removeprefix("sha256:")
        output = run(client, vm["id"], guest_probe(expected, victim), target="vm")
        (tmp_path / "guest-probe.txt").write_text(output)
        seen = fields(output)
        assert set(seen["LIST"].split()) == expected, output
        assert seen["HASHES"].split() == sorted(expected), "the guest reads the verified bytes"
        assert set(seen["AFTER"].split()) == expected, output
        assert "WROTE:" not in output, output
        # The host's copy is untouched, whatever the guest tried.
        assert {path.name for path in share.iterdir()} == expected
        assert hashlib.sha256((share / victim).read_bytes()).hexdigest() == victim

        # A fork links its source's blobs; nothing comes back from the guest.
        fork_id = client.post(f"/vms/{vm['id']}/fork", {"name": "image-share-fork"})["id"]
        try:
            fork = vm_session_dir(service.tmp_dir, client, fork_id)
            assert {path.name for path in share_of(fork).iterdir()} == expected
            for name in expected:
                assert (share_of(fork) / name).stat().st_ino == (share / name).stat().st_ino, name
            assert_stage_holds_no_image(fork, document["layers"])
        finally:
            client.delete(f"/vms/{fork_id}/delete")
