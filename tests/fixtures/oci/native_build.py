"""Fixed native recipes use the original producer and one verified shared base."""

from __future__ import annotations

import json
import subprocess
from contextlib import contextmanager

from capsem_builder.cache import dockercurrent
from capsem_builder.cache.config import load_paths
from capsem_builder.cache.leases import release_path, retain_generation

from . import image_build
from .image_build import _retained_image
from .native_candidates import host_platform
from .native_inputs import BaseBinding, BuildBinding, metadata, workflow_key
from .pinned_image import TIMEOUT, PinnedImage
from .producer_evidence import read_file, safe_tree


class CandidateImage(PinnedImage):
    def __init__(self, config, name):
        if name != "dev" and name not in config.functional.native_images.apps:
            raise ValueError("native image is not an allowlisted candidate")
        super().__init__("reference_image" if name == "dev" else name)
        self.config, self.name = config, name

    def settings(self):
        if self.name == "dev":
            return self.config.functional.reference_image
        return self.config.functional.native_images.apps[self.name]

    def stage(self):
        return load_paths(self.config.root).stage(self.settings().cache_stage)

    def pinned(self, selected=None):
        raise ValueError("a native build candidate is not a functional digest pin")


def candidate(config, name):
    return CandidateImage(config, name)


def loaded(tag, platform, manifest, selected, *, subject):
    identity, config, layers = _retained_image("docker", tag, platform.split("/")[1], TIMEOUT,
        expected_manifest=subject, expected_config=manifest["config"]["digest"])
    if config != manifest["config"]["digest"] or layers != selected["rootfs"]["diff_ids"]:
        raise ValueError("native Docker config or ordered layers differ from original OCI")
    return identity


@contextmanager
def shared_base(config, platform):
    settings = config.functional.native_images
    if platform != settings.platform:
        raise ValueError("native base platform differs")
    spec = settings.base
    key = workflow_key(config.root, "base")
    generation = "native-base-" + platform.split("/")[1] + "-" + key
    paths = load_paths(config.root)
    template = paths.policy.stages[spec.cache_stage].lease_template
    if template is None:
        raise ValueError("native base stage has no lease control")
    retain_generation(paths, spec.cache_stage, generation)
    root = paths.stage(spec.cache_stage)
    lease = root / template.format(key=generation)
    target = root / generation
    try:
        target.mkdir(exist_ok=True)
        if target.is_symlink():
            raise ValueError("native base generation is a symlink")
        layout = target / "layout"
        common = [
            "docker",
            "buildx",
            "build",
            "--platform",
            platform,
            "--provenance=false",
            "--sbom=false",
        ]
        context = str(config.root / spec.context)
        if not layout.exists():
            partial = target / "partial"
            if partial.exists() or partial.is_symlink():
                raise ValueError("native base has incomplete original output")
            subprocess.run(
                [*common, "--output", f"type=oci,tar=false,dest={partial}", context],
                check=True,
                timeout=3600,
            )
            safe_tree(partial)
            (entry,) = json.loads(read_file(partial / "index.json", settings.receipt_max_bytes))[
                "manifests"
            ]
            BaseBinding.capture(
                partial, entry["digest"], platform, key, maximum=settings.receipt_max_bytes
            )
            partial.rename(layout)
        safe_tree(layout)
        (entry,) = json.loads(read_file(layout / "index.json", settings.receipt_max_bytes))[
            "manifests"
        ]
        binding = BaseBinding.capture(
            layout, entry["digest"], platform, key, maximum=settings.receipt_max_bytes
        )
        manifest, selected = binding.decoded(platform)
        tag = spec.local_name + ":" + platform.split("/")[1]
        dockercurrent.record(
            paths, tag=tag, checkout=config.root, slot="native-base-" + platform.split("/")[1]
        )
        subprocess.run([*common, "--load", "--tag", tag, context], check=True, timeout=3600)
        yield loaded(tag, platform, manifest, selected, subject=binding.subject), binding
    finally:
        release_path(lease)


@contextmanager
def produce(config, name, platform):
    if host_platform() != platform or platform != config.functional.native_images.platform:
        raise ValueError("native candidate requires the actual configured native host")
    image = candidate(config, name)
    maximum = config.functional.native_images.receipt_max_bytes
    with shared_base(config, platform) as (base, binding):
        inputs = BuildBinding(
            input_key=workflow_key(config.root, name, binding.subject), base=binding
        )
        paths = load_paths(config.root)
        template = paths.policy.stages[image.settings().cache_stage].lease_template
        if template is None:
            raise ValueError("native candidate stage has no lease control")
        tag = image.settings().local_name + ":" + platform.split("/")[1]
        dockercurrent.record(
            paths,
            tag=tag,
            checkout=config.root,
            slot="native-" + name + "-" + platform.split("/")[1],
        )

        held = None
        try:

            def validate(layout, digest, selected_platform, selected_tag):
                nonlocal held
                key = image.layout_path(digest).name
                retain_generation(paths, image.settings().cache_stage, key)
                held = paths.stage(image.settings().cache_stage) / template.format(key=key)
                inputs.require(config.root, name, layout, digest, selected_platform, maximum)
                _, _, manifest, selected = metadata(layout, digest, platform, maximum)
                loaded(selected_tag, platform, manifest, selected, subject=digest)

            digest = image_build.build(image, platform, base_image=base, validate=validate)
            if held is None:
                raise ValueError("native producer did not validate its original output")
            inputs.require(config.root, name, image.layout_path(digest), digest, platform, maximum)
            yield image, digest, inputs
        finally:
            if held is not None:
                release_path(held)
