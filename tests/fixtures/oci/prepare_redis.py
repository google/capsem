"""Explicit host-only prefetch for the Redis spike; never starts the image."""

import gzip
import hashlib
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUTPUT = ROOT / "cache/target/tests/redis-image"


def docker(*args):
    return subprocess.run(
        ["docker", *args], check=True, capture_output=True, text=True, timeout=180
    ).stdout


def main():
    pin = json.loads(Path(__file__).with_name("redis-image.json").read_text())
    docker("pull", "--platform", pin["platform"], pin["image"])
    image = json.loads(docker("image", "inspect", pin["image"]))[0]
    assert f"{image['Os']}/{image['Architecture']}" == pin["platform"]
    OUTPUT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=OUTPUT) as tmp:
        container = docker(
            "create",
            "--platform",
            pin["platform"],
            "--network",
            "none",
            "--entrypoint",
            "/bin/true",
            pin["image"],
        ).strip()
        try:
            exported = Path(tmp) / "rootfs.tar"
            docker("export", "--output", str(exported), container)
        finally:
            docker("rm", "--volumes", container)
        archive = Path(tmp) / "redis-rootfs.tar.gz"
        with (
            exported.open("rb") as source,
            archive.open("wb") as target,
            gzip.GzipFile(fileobj=target, mode="wb", mtime=0, filename="") as gz,
        ):
            shutil.copyfileobj(source, gz)
        metadata = {
            **pin,
            "image_id": image["Id"],
            "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
        }
        archive.replace(OUTPUT / archive.name)
        (OUTPUT / "redis-image.json").write_text(json.dumps(metadata, indent=2))
        print(json.dumps(metadata, indent=2))


if __name__ == "__main__":
    main()
