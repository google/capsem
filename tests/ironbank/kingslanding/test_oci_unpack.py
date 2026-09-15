"""Packaged umoci must perform real OCI layer and configuration unpacking."""

import importlib.util
import platform

from tests.ironbank.kingslanding.test_oci_container import FIXTURES, oci_vm
from tests.ironbank.kingslanding.test_run import exec_output_text

__all__ = ["oci_vm"]


def test_packaged_umoci_unpacks_layer_semantics(oci_vm, tmp_path):
    spec = importlib.util.spec_from_file_location("oci_image_layout", FIXTURES / "image_layout.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    layout = tmp_path / "layout"
    architecture = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "amd64"}[platform.machine()]
    digest = module.write_layout(layout, architecture)
    _, client, name = oci_vm
    for path in sorted(layout.rglob("*")):
        if path.is_file():
            data = path.read_bytes()
            response = client.post_bytes(
                f"/vms/{name}/files/content?path=oci-unpack/{path.relative_to(layout)}", data
            )
            assert response == {"success": True, "size": len(data)}
    probe = (FIXTURES / "unpack_probe.py").read_bytes()
    assert client.post_bytes(f"/vms/{name}/files/content?path=unpack_probe.py", probe) == {
        "success": True, "size": len(probe)
    }
    result = client.post(
        f"/vms/{name}/exec",
        {"command": "umoci --version && python3 /root/unpack_probe.py", "timeout_secs": 30},
    )
    assert result["exit_code"] == 0, result
    stdout = exec_output_text(result)
    assert "OCI_UNPACK: whiteouts,binary,symlink,entrypoint,cmd=PASS" in stdout
    (tmp_path / "identity.txt").write_text(f"{architecture}\n{digest}\n{stdout}")


def test_unpack_profile_preserves_guest_hardening(oci_vm, tmp_path):
    _, client, name = oci_vm
    result = client.post(
        f"/vms/{name}/exec",
        {"command": "capsem-doctor -q -k 'guest_binary_not_writable or "
         "no_real_nics or rootfs_block_device_is_immutable'", "timeout_secs": 60},
        timeout=75,
    )
    stdout = exec_output_text(result)
    (tmp_path / "hardening.txt").write_text(stdout + exec_output_text(result, "stderr"))
    assert result["exit_code"] == 0, result
    assert "passed" in stdout, result
    assert "skipped" not in stdout, result
