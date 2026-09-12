"""Explicit host-only prefetch of the pinned test images; never starts one.

Each image has a pins file beside this script (`<image>-image.json`) and
lands in the output directory as `<image>-image.json` plus
`<image>-rootfs.tar.gz`. Redis is the container under test; iperf3 is the
native reference for the private-path performance matrix.
"""

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


IMAGES = ("redis", "iperf3")


def native_pin(selected=None, image="redis"):
    if selected is None:
        arch = {
            "arm64": "arm64",
            "aarch64": "arm64",
            "x86_64": "amd64",
            "AMD64": "amd64",
        }.get(platform.machine())
        selected = f"linux/{arch}"
    if image not in IMAGES:
        raise ValueError(f"unknown fixture image: {image}")
    pins = json.loads(Path(__file__).with_name(f"{image}-image.json").read_text())
    if selected not in pins["images"]:
        raise ValueError(f"unsupported {image} fixture platform: {selected}")
    return {
        "tag": pins["tag"],
        "version": pins["version"],
        "platform": selected,
        "image": pins["images"][selected],
    }


def cached(output, pin, image="redis"):
    try:
        metadata = json.loads((output / f"{image}-image.json").read_text())
        archive = (output / f"{image}-rootfs.tar.gz").read_bytes()
    except FileNotFoundError:
        return False
    return all(
        metadata.get(key) == value for key, value in pin.items()
    ) and hashlib.sha256(archive).hexdigest() == metadata.get("archive_sha256")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--platform")
    parser.add_argument("--image", action="append", choices=IMAGES)
    args = parser.parse_args()
    for image in args.image or IMAGES:
        prepare(args.output, image, args.platform)


def prepare(output, image, platform_name=None):
    pin = native_pin(platform_name, image)
    if cached(output, pin, image):
        print(f"Verified cached {image} fixture: {pin['image']}")
        return
    docker("pull", "--platform", pin["platform"], pin["image"])
    inspected = json.loads(docker("image", "inspect", pin["image"]))[0]
    assert f"{inspected['Os']}/{inspected['Architecture']}" == pin["platform"]
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
        archive = Path(tmp) / f"{image}-rootfs.tar.gz"
        with (
            exported.open("rb") as source,
            archive.open("wb") as target,
            gzip.GzipFile(fileobj=target, mode="wb", mtime=0, filename="") as gz,
        ):
            shutil.copyfileobj(source, gz)
        metadata = {
            **pin,
            "image_id": inspected["Id"],
            "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
        }
        archive.replace(output / archive.name)
        (output / f"{image}-image.json").write_text(json.dumps(metadata, indent=2))
        print(json.dumps(metadata, indent=2))


if __name__ == "__main__":
    main()
