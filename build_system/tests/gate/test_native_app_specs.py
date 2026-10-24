"""Native candidate recipes are allowlisted build inputs, never invented pins."""

from copy import deepcopy
from pathlib import Path

import pytest
from capsem_builder.gate.config import load
from capsem_builder.gate.functionalschema import NativeImagesConfig, PinnedImageConfig
from pydantic import ValidationError

ROOT = Path(__file__).resolve().parents[3]


def test_fixed_apps_extend_native_builds_without_changing_functional_pins():
    config = load(ROOT)
    native = config.functional.native_images
    assert set(native.apps) == {"codex-cli", "claude-code", "agy", "antigravity-desktop"}
    assert native.base.context == "images/base" and native.base.base_context is None
    for name, spec in native.apps.items():
        assert spec.name == name and spec.context == "images/" + name
        assert spec.base_context == native.base.context
        assert spec.repository == "ghcr.io/google/capsem/" + name
        assert spec.cache_stage == "test-images" and "digests" not in spec.model_dump()
    assert config.functional.reference_image.name == "dev"
    assert config.functional.debug_image.name == "capsem-debug"
    for settings in (config.functional.reference_image, config.functional.debug_image):
        with pytest.raises(ValidationError):
            PinnedImageConfig.model_validate(settings.model_dump() | {"digests": {}})


@pytest.mark.parametrize(
    "fault",
    [
        "missing",
        "unknown",
        "claude-amd",
        "context",
        "repository",
        "base",
        "pin",
        "base-context",
        "tag",
        "script",
        "cache",
    ],
)
def test_native_specs_reject_missing_foreign_or_unpinned_substitutes(fault):
    data = deepcopy(load(ROOT).functional.native_images.model_dump())
    app = data["apps"]["codex-cli"]
    if fault == "missing":
        del data["apps"]["agy"]
    elif fault in {"unknown", "claude-amd"}:
        data["apps"]["unknown" if fault == "unknown" else "claude-desktop"] = app
    elif fault == "context":
        app["context"] = "../foreign"
    elif fault == "repository":
        app["repository"] = "ghcr.io/foreign/codex-cli"
    elif fault == "base":
        app["base_context"] = "images/dev"
    elif fault == "pin":
        app["digests"] = {"linux/amd64": "sha256:" + "a" * 64}
    elif fault == "tag":
        app["local_name"] = "foreign"
    elif fault == "script":
        app["script"] = "foreign.py"
    elif fault == "cache":
        app["cache_stage"] = "foreign"
    else:
        data["base"]["context"] = "images/dev"
    with pytest.raises(ValidationError):
        NativeImagesConfig.model_validate(data)


@pytest.mark.parametrize("field", ["context_root", "script"])
@pytest.mark.parametrize("value", ["/foreign", "../foreign", ""])
def test_native_recipe_policy_rejects_escaping_paths(field, value):
    data = deepcopy(load(ROOT).functional.native_images.model_dump())
    data["recipe"][field] = value
    with pytest.raises(ValidationError):
        NativeImagesConfig.model_validate(data)
