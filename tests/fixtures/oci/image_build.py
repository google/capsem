"""Build a pinned image and its prepared filesystem through the canonical producers."""

import json
import math
import re
import subprocess
import tempfile
from pathlib import Path

from .pinned_image import ROOT, _publish, verify


def build(image, selected: str | None = None, *, base_image: str | None = None, validate=None) -> str:
    """Build into the digest stage and load the configured owned local Docker tag.

    An image built FROM a base (`base_context`) gets that base built first
    and passed as its BASE argument, as the image workflow does.
    """
    settings = image.settings()
    platform = image.platform_name(selected)
    arch = platform.split("/")[1]
    tag = f"{settings.local_name}:{arch}"
    common = [
        "docker",
        "buildx",
        "build",
        "--platform",
        platform,
        "--provenance=false",
        "--sbom=false",
    ]
    if settings.base_context and base_image is None:
        base = f"{settings.local_name}-base:{arch}"
        subprocess.run(
            [*common, "--load", "--tag", base, str(ROOT / settings.base_context)],
            check=True,
            timeout=3600,
        )
        common += ["--build-arg", f"BASE={base}"]
    elif base_image is not None:
        common += ["--build-arg", f"BASE={base_image}"]
    context = str(ROOT / settings.context)
    image.stage().mkdir(parents=True, exist_ok=True)
    from images.ci.rootfs import main as build_rootfs

    from . import producer_evidence

    with tempfile.TemporaryDirectory(dir=image.stage(), prefix=".build-") as tmp:
        staged = Path(tmp) / "layout"
        output = f"type=oci,tar=false,dest={staged}"
        subprocess.run([*common, "--output", output, context], check=True, timeout=3600)
        (entry,) = json.loads((staged / "index.json").read_text())["manifests"]
        digest = entry["digest"]
        producer_evidence.safe_tree(staged)
        verify(staged, digest, platform)
        layout = image.layout_path(digest)
        if layout.exists() or layout.is_symlink():
            # Reuse first producer bytes; missing metadata is a refusal, never
            # an excuse to regenerate a different filesystem under this subject.
            producer_evidence.ready(layout, settings.repository, arch, maximum=settings.evidence_max_bytes)
            subprocess.run([*common, "--load", "--tag", tag, context], check=True, timeout=3600)
            if validate is not None:
                validate(staged, digest, platform, tag)
        else:
            subprocess.run([*common, "--load", "--tag", tag, context], check=True, timeout=3600)
            if validate is not None:
                validate(staged, digest, platform, tag)
            filesystem = Path(tmp) / "filesystem"
            filesystem.mkdir()
            build_rootfs(["--image", tag, "--arch", arch, "--repository", settings.repository,
                "--digest", digest, "--out", str(filesystem),
                *(["--without-shared-toolkit"] if image.section == "debug_image" else [])])
            evidence = producer_evidence.stage(staged, filesystem, settings.repository, arch, maximum=settings.evidence_max_bytes)
            expected = (evidence / "receipt.json").read_bytes()
            _publish(staged, layout)
            actual = producer_evidence.ready(layout, settings.repository, arch, maximum=settings.evidence_max_bytes)
            if (actual / "receipt.json").read_bytes() != expected:
                raise ValueError("published generation differs from original producer evidence")
    print(f"Built {settings.name} {platform}: {digest}")
    print(f"  layout: {layout}")
    print(f"  docker: {tag}")
    print(f"  original evidence: {layout / producer_evidence.DIRECTORY}")
    if validate is None:
        print(f"Pin it in config/gate.toml [functional.{image.section}] digests:")
        print(f'  "{platform}" = "{digest}"')
    else:
        print("Native candidate only; functional digest pins unchanged.")
    return digest


def _retained_image(runtime, tag, architecture, inspect_timeout, *, config_required=True,
                    expected_manifest=None, expected_config=None):
    fields = '{"Id":{{json .Id}},"Descriptor":{{json .Descriptor}},"Os":{{json .Os}},"Architecture":{{json .Architecture}},"RootFS":{{json .RootFS}}}'
    found = subprocess.run([runtime, "image", "inspect", "--format", fields, tag],
                           capture_output=True, text=True, check=False, timeout=inspect_timeout)
    if found.returncode != 0:
        raise ValueError(f"missing retained preparation image: {tag}")
    value = json.loads(found.stdout)
    if not isinstance(value, dict):
        raise ValueError("retained image identity is invalid")
    identity = value.get("Id")
    if not isinstance(identity, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", identity):
        raise ValueError("retained image identity is invalid")
    if (value.get("Os"), value.get("Architecture")) != ("linux", architecture):
        raise ValueError("retained image platform differs from selected image")
    descriptor = value.get("Descriptor")
    config = identity
    if descriptor is not None:
        media = descriptor.get("mediaType") if isinstance(descriptor, dict) else None
        manifest = media in {"application/vnd.oci.image.manifest.v1+json", "application/vnd.docker.distribution.manifest.v2+json"}
        tools_index = not config_required and media in {"application/vnd.oci.image.index.v1+json", "application/vnd.docker.distribution.manifest.list.v2+json"}
        if not isinstance(descriptor, dict) or descriptor.get("digest") != identity or not (manifest or tools_index):
            raise ValueError("retained image descriptor identity is invalid")
        annotations = descriptor.get("annotations")
        config = annotations.get("config.digest") if isinstance(annotations, dict) else None
        # Containerd's immutable image ID can name the OCI manifest.
        # Only that exact verified manifest binds its original config;
        # an unrelated image without the optional annotation still fails.
        if config_required and config is None and identity == expected_manifest and (
            annotations is None or (isinstance(annotations, dict) and "config.digest" not in annotations)
        ):
            config = expected_config
        if (config_required or config is not None) and (not isinstance(config, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", config)):
            raise ValueError("retained image descriptor config identity is invalid")
    rootfs = value.get("RootFS")
    layers = rootfs.get("Layers") if isinstance(rootfs, dict) else None
    if not isinstance(rootfs, dict) or rootfs.get("Type") != "layers" or not isinstance(layers, list) or not layers or any(
        not isinstance(layer, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", layer) for layer in layers
    ):
        raise ValueError("retained image layers are invalid")
    return identity, config, layers


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
    selected = json.loads((layout / "blobs/sha256" / manifest["config"]["digest"].split(":")[1]).read_text())
    identity, config, layers = _retained_image(rootfs.RUNTIME, f"{image.settings().local_name}:{architecture}", architecture, inspect_timeout,
        expected_manifest=entry["digest"], expected_config=manifest["config"]["digest"])
    if config != manifest["config"]["digest"]:
        raise ValueError("retained image identity differs from selected OCI config")
    expected_root = selected.get("rootfs")
    if not isinstance(expected_root, dict) or expected_root.get("type") != "layers" or layers != expected_root.get("diff_ids"):
        raise ValueError("retained image ordered layers differ from selected OCI config")
    build = rootfs.build_config()
    arch_name = rootfs.builder_architecture(build, architecture)
    # The input-keyed tools may be an index. Its immutable digest and selected
    # platform are sufficient; never mistake that index for a workload config.
    tools, _, _ = _retained_image(rootfs.RUNTIME, rootfs.assettools.image_tag(build, arch_name, rootfs.ROOT), architecture, inspect_timeout, config_required=False)
    rootfs.main(["--image", identity, "--arch", architecture, "--repository", reference.split("@")[0],
                 "--digest", entry["digest"], "--out", str(output)], prepared_tools=tools)
    read_evidence(output, reference, architecture, entry["digest"])
    return output
