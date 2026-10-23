"""Build a pinned image and its prepared filesystem through the canonical producers."""

import json
import subprocess
import tempfile
from pathlib import Path

from .pinned_image import ROOT, _publish, verify


def build(image, selected: str | None = None) -> str:
    """Build the image into the stage by digest and load it as `<name>:<arch>` too.

    An image built FROM a base (`base_context`) gets that base built first
    and passed as its BASE argument, as the image workflow does.
    """
    settings = image.settings()
    platform = image.platform_name(selected)
    arch = platform.split("/")[1]
    common = [
        "docker",
        "buildx",
        "build",
        "--platform",
        platform,
        "--provenance=false",
        "--sbom=false",
    ]
    if settings.base_context:
        base = f"{settings.name}-base:{arch}"
        subprocess.run(
            [*common, "--load", "--tag", base, str(ROOT / settings.base_context)],
            check=True,
            timeout=3600,
        )
        common += ["--build-arg", f"BASE={base}"]
    context = str(ROOT / settings.context)
    image.stage().mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=image.stage(), prefix=".build-") as tmp:
        staged = Path(tmp) / "layout"
        output = f"type=oci,tar=false,dest={staged}"
        subprocess.run([*common, "--output", output, context], check=True, timeout=3600)
        (entry,) = json.loads((staged / "index.json").read_text())["manifests"]
        digest = entry["digest"]
        verify(staged, digest, platform)
        layout = image.layout_path(digest)
        if layout.is_dir():
            verify(layout, digest, platform)
        else:
            _publish(staged, layout)
    # The same build again from BuildKit's cache, loaded into Docker under a
    # local tag, for images/ci/rootfs.py (the OBOM) and hands-on use.
    tag = f"{settings.name}:{arch}"
    subprocess.run([*common, "--load", "--tag", tag, context], check=True, timeout=3600)
    from images.ci.rootfs import main as build_rootfs

    from .published_root import stage

    with tempfile.TemporaryDirectory(dir=image.stage(), prefix=".filesystem-") as temporary:
        output = Path(temporary)
        build_rootfs(
            [
                "--image",
                tag,
                "--arch",
                arch,
                "--repository",
                settings.repository,
                "--digest",
                digest,
                "--out",
                str(output),
                *(["--without-shared-toolkit"] if image.section == "debug_image" else []),
            ]
        )
        stage(layout, output)
    print(f"Built {settings.name} {platform}: {digest}")
    print(f"  layout: {layout}")
    print(f"  docker: {tag}")
    print(f"Pin it in config/gate.toml [functional.{image.section}] digests:")
    print(f'  "{platform}" = "{digest}"')
    return digest
