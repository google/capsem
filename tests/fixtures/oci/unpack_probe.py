"""Assert OCI whiteout, binary content, symlink and image-command semantics."""

import json
import subprocess
import tempfile
from pathlib import Path

with tempfile.TemporaryDirectory(prefix="oci-unpack-", dir="/var/tmp") as temporary:
    bundle = Path(temporary) / "bundle"
    result = subprocess.run(
        ["umoci", "unpack", "--image", "/root/oci-unpack:image", str(bundle)],
        capture_output=True,
        text=True,
        timeout=20,
        check=True,
    )
    rootfs = bundle / "rootfs"
    assert (rootfs / "keep").read_bytes() == b"kept"
    assert not (rootfs / "delete").exists()
    assert not (rootfs / "dir/old").exists()
    assert (rootfs / "dir/new").read_bytes() == bytes(range(256))
    assert str((rootfs / "link").readlink()) == "/keep"
    assert not list(rootfs.rglob(".wh.*"))
    config = json.loads((bundle / "config.json").read_text())
    assert config["process"]["args"] == ["/bin/app", "serve"]
    print("OCI_UNPACK: whiteouts,binary,symlink,entrypoint,cmd=PASS")
