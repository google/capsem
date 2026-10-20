"""Vectors and corruption cases for the owned Python integrity bindings."""

import io
import subprocess
import sys

import pytest
from capsem_builder import crypto


def test_sha256_works_without_site_packages_and_blake3_fails_explicitly():
    code = (
        "import importlib.util, sys; "
        "spec = importlib.util.spec_from_file_location('crypto', sys.argv[1]); "
        "owner = importlib.util.module_from_spec(spec); spec.loader.exec_module(owner); "
        "print(owner.sha256(b'abc').hexdigest()); "
        "owner.blake3(b'abc')"
    )
    result = subprocess.run(
        [sys.executable, "-S", "-c", code, crypto.__file__],
        env={}, capture_output=True, text=True, check=False, timeout=10,
    )
    assert result.stdout.strip() == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    assert result.returncode == 1
    assert "ModuleNotFoundError: No module named 'blake3'" in result.stderr


@pytest.mark.parametrize(
    ("algorithm", "expected"),
    [
        ("sha256", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        ("blake3", "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"),
    ],
)
def test_file_and_reader_vectors_and_corruption(tmp_path, algorithm, expected):
    path = tmp_path / "content"
    path.write_bytes(b"abc")
    assert crypto.digest_file(path, algorithm=algorithm) == expected
    crypto.verify_reader(io.BytesIO(b"abc"), f"{algorithm}:{expected}", 3)
    for data in [b"abd", b"ab", b"abcd"]:
        with pytest.raises(ValueError):
            crypto.verify_reader(io.BytesIO(data), f"{algorithm}:{expected}", 3)


def test_bounds_and_malformed_descriptors_fail_before_unbounded_reads():
    source = io.BytesIO(b"x" * (2 << 20))
    with pytest.raises(ValueError):
        crypto.verify_reader(source, "blake3:" + "0" * 64, 3)
    assert source.tell() == 4
    for digest, size in [
        ("blake3:../root", 3),
        ("sha512:" + "0" * 64, 3),
        ("blake3:" + "A" * 64, 3),
        ("blake3:" + "0" * 64, -1),
        ("blake3:" + "0" * 64, True),
    ]:
        with pytest.raises(ValueError):
            crypto.verify_reader(io.BytesIO(b"abc"), digest, size)
