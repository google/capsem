"""Build a pinned image and its prepared filesystem through the canonical producers."""

import json
import math
import re
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


def prepare_local_evidence(image, reference: str, output: Path, *, inspect_timeout: float) -> Path:
    """Untimed official callback; caller retains normal image/cache leases.

    Missing retained images/tools fail rather than building or fetching them.
    Immutable Docker IDs survive tag changes during export/scan/repack. The
    canonical producer still owns all package/OBOM generation and validation.
    """
    from images.ci import rootfs
    from tests.fixtures.oci.catalog_evidence import read_evidence

    if not math.isfinite(inspect_timeout) or inspect_timeout <= 0 or output.exists():
        raise ValueError("preparation needs a finite positive budget and fresh output")
    layout = image.ready()
    (entry,) = json.loads((layout / "index.json").read_text())["manifests"]
    if entry["digest"] != image.pinned():
        raise ValueError("prepared layout identity differs from selected pin")
    architecture = image.platform_name().split("/")[1]
    name = image.settings().name
    local = re.fullmatch(r"127\.0\.0\.1:([1-9][0-9]*)/library/" + re.escape(name) + "@" + re.escape(entry["digest"]), reference)
    if not local or int(local[1]) > 65535:
        raise ValueError("evidence requires this exact local image identity")
    manifest = json.loads((layout / "blobs/sha256" / entry["digest"].split(":")[1]).read_text())
    def retained(tag):
        found = subprocess.run([rootfs.RUNTIME, "image", "inspect", "--format", "{{json .Id}}", tag],
                               capture_output=True, text=True, check=False, timeout=inspect_timeout)
        if found.returncode != 0:
            raise ValueError(f"missing retained preparation image: {tag}")
        identity = json.loads(found.stdout)
        if not isinstance(identity, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", identity):
            raise ValueError("retained image identity is invalid")
        return identity
    identity = retained(f"{name}:{architecture}")
    if identity != manifest["config"]["digest"]:
        raise ValueError("retained image identity differs from selected OCI config")
    build = rootfs.build_config()
    arch_name = rootfs.builder_architecture(build, architecture)
    tools = retained(rootfs.assettools.image_tag(build, arch_name, rootfs.ROOT))
    rootfs.main(["--image", identity, "--arch", architecture, "--repository", reference.split("@")[0],
                 "--digest", entry["digest"], "--out", str(output)], prepared_tools=tools)
    read_evidence(output, reference, architecture, entry["digest"])
    return output
