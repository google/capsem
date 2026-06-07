"""Winterfell fork-lineage E2E proof.

Boot a real VM, write a file, fork it, delete the source, boot the fork, mutate
state, fork again, delete the middle VM, and prove the final fork has the exact
expected filesystem state.
"""

import uuid

import pytest

from helpers.profile_asset_fixture import (
    asset_source_dir,
    find_asset,
    write_profile_home,
)

pytestmark = [pytest.mark.e2e, pytest.mark.serial]


def _name(prefix: str) -> str:
    return f"{prefix}-{uuid.uuid4().hex[:8]}"


def _exec_ok(service, vm: str, command: str, *, timeout: int = 90):
    return service.cli_ok("exec", vm, command, timeout=timeout)


def _assert_ready(service, vm: str):
    assert service.wait_exec_ready(vm, timeout=180), f"VM {vm} never exec-ready"


def test_winterfell_fork_lineage_survives_delete_resume_and_refork(
    tmp_path, real_service_factory
):
    source_dir = asset_source_dir()
    if not source_dir.exists():
        pytest.skip(f"asset source dir missing: {source_dir}")

    assets = {
        "vmlinuz": find_asset(source_dir, "vmlinuz"),
        "initrd.img": find_asset(source_dir, "initrd.img"),
        "rootfs.squashfs": find_asset(source_dir, "rootfs.squashfs"),
    }
    capsem_home = tmp_path / "capsem-home"
    asset_cache = tmp_path / "downloaded-assets"
    write_profile_home(capsem_home, asset_cache, assets)
    service = real_service_factory(capsem_home=capsem_home, assets_dir=asset_cache)
    vm1 = _name("wf-one")
    vm2 = _name("wf-two")
    vm3 = _name("wf-three")
    path = "/root/winterfell"
    file1 = f"{path}/oath.txt"
    file2 = f"{path}/raven.txt"
    created = []

    try:
        service.start()
        update = service.cli_ok("update", "--assets", timeout=240)
        assert "Profile VM assets reconciled" in update.stdout or "already ready" in update.stdout

        service.cli_ok("create", vm1, timeout=180)
        created.append(vm1)
        _assert_ready(service, vm1)
        _exec_ok(
            service,
            vm1,
            f"mkdir -p {path} && printf 'winterfell-file-one\\n' > {file1}",
        )

        service.cli_ok("fork", vm1, vm2, timeout=180)
        created.append(vm2)
        service.cli_ok("delete", vm1, timeout=120)
        created.remove(vm1)

        service.cli_ok("resume", vm2, timeout=180)
        _assert_ready(service, vm2)
        read_file1 = _exec_ok(service, vm2, f"cat {file1}")
        assert read_file1.stdout == "winterfell-file-one\n"

        _exec_ok(
            service,
            vm2,
            f"rm {file1} && printf 'winterfell-file-two\\n' > {file2} && test ! -e {file1}",
        )

        service.cli_ok("fork", vm2, vm3, timeout=180)
        created.append(vm3)
        service.cli_ok("delete", vm2, timeout=120)
        created.remove(vm2)

        service.cli_ok("resume", vm3, timeout=180)
        _assert_ready(service, vm3)
        read_file2 = _exec_ok(service, vm3, f"test ! -e {file1} && cat {file2}")
        assert read_file2.stdout == "winterfell-file-two\n"
    finally:
        for vm in reversed(created):
            service.cli("delete", vm, timeout=120)
        service.stop()
