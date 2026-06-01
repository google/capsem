import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from hypervisor_bench_common import benchmark_init_script, extract_json  # noqa: E402


def test_hypervisor_benchmark_init_script_uses_engine_specific_markers():
    script = benchmark_init_script(marker_prefix="CROSVM", log_prefix="crosvm-bench")

    assert "[crosvm-bench] rootfs benchmark start" in script
    assert "CAPSEM_CROSVM_ROOTFS_JSON_BEGIN" in script
    assert "CAPSEM_CROSVM_STARTUP_JSON_END" in script
    assert "CAPSEM_FIRECRACKER_ROOTFS_JSON_BEGIN" not in script


def test_extract_json_reads_named_marker_payload():
    payload = {"rootfs": {"seq_read": {"mbps": 123.4}}}
    serial = (
        "noise\n"
        "CAPSEM_CROSVM_ROOTFS_JSON_BEGIN\n"
        f"{json.dumps(payload)}\n"
        "CAPSEM_CROSVM_ROOTFS_JSON_END\n"
    )

    assert extract_json(serial, "CROSVM", "ROOTFS") == payload
