"""Explicit host-only prefetch for the Redis spike; never starts the image."""

import argparse
import gzip
import hashlib
import json
import platform
import shutil
import subprocess
import tempfile
from pathlib import Path


def docker(*args):
    return subprocess.run(
        ["docker", *args], check=True, capture_output=True, text=True, timeout=180
    ).stdout


def native_pin(selected=None):
    if selected is None:
        arch = {
            "arm64": "arm64",
            "aarch64": "arm64",
            "x86_64": "amd64",
            "AMD64": "amd64",
        }.get(platform.machine())
        selected = f"linux/{arch}"
    pins = json.loads(Path(__file__).with_name("redis-image.json").read_text())
    if selected not in pins["images"]:
        raise ValueError(f"unsupported Redis fixture platform: {selected}")
    return {
        "tag": pins["tag"],
        "version": pins["version"],
        "platform": selected,
        "image": pins["images"][selected],
    }


def cached(output, pin):
    try:
        metadata = json.loads((output / "redis-image.json").read_text())
        archive = (output / "redis-rootfs.tar.gz").read_bytes()
    except FileNotFoundError:
        return False
    return all(
        metadata.get(key) == value for key, value in pin.items()
    ) and hashlib.sha256(archive).hexdigest() == metadata.get("archive_sha256")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--platform")
    args = parser.parse_args()
    output = args.output
    pin = native_pin(args.platform)
    if cached(output, pin):
        print(f"Verified cached Redis fixture: {pin['image']}")
        return
    docker("pull", "--platform", pin["platform"], pin["image"])
    image = json.loads(docker("image", "inspect", pin["image"]))[0]
    assert f"{image['Os']}/{image['Architecture']}" == pin["platform"]
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output) as tmp:
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
        archive.replace(output / archive.name)
        (output / "redis-image.json").write_text(json.dumps(metadata, indent=2))
        print(json.dumps(metadata, indent=2))


if __name__ == "__main__":
    main()
