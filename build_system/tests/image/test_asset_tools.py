"""The offline filesystem packer must be selected by source digest, not APT age."""

from pathlib import Path

import pytest
from capsem_builder.image import assettools
from capsem_builder.image.config import load_guest_config

ROOT = Path(__file__).resolve().parents[3]


def test_asset_tools_build_arguments_pin_the_erofs_source() -> None:
    build = load_guest_config(ROOT / "config/docker/image").build
    source = build.asset_tools.erofs_source
    args = assettools.build_arguments(build, "x86_64", "test-identity")
    assert f"EROFS_SOURCE_URL={source.url}" in args
    assert f"EROFS_SOURCE_SHA256={source.sha256}" in args


@pytest.mark.parametrize("field,value", [("url", "https://example.test/erofs.tar.gz"), ("sha256", "0" * 64)])
def test_each_erofs_source_input_changes_the_asset_tools_cache_key(field: str, value: str) -> None:
    build = load_guest_config(ROOT / "config/docker/image").build
    source = build.asset_tools.erofs_source.model_copy(update={field: value})
    changed = build.model_copy(update={"asset_tools": build.asset_tools.model_copy(update={"erofs_source": source})})
    assert assettools.image_tag(build, "x86_64", ROOT) != assettools.image_tag(changed, "x86_64", ROOT)
