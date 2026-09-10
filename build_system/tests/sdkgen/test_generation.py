"""Generation drift cannot omit new, stale, deleted or manually edited modules."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path

import pytest
from capsem_builder.sdkgen.generation import python_sources, synchronize, typescript_sources

SPEC = Path(__file__).resolve().parents[3] / "sdk/specification/openapi.json"


@pytest.mark.parametrize("render", [python_sources, typescript_sources])
def test_generation_is_deterministic_and_check_does_not_write(
    tmp_path: Path, render: Callable[[Path], dict[str, dict[str, str]]],
) -> None:
    sources = render(SPEC)
    assert sources == render(SPEC)
    package = tmp_path / "package"
    missing = synchronize(package, sources, check=True)
    assert len(missing) == sum(len(files) for files in sources.values())
    assert not package.exists()
    assert synchronize(package, sources, check=False) == missing
    assert synchronize(package, sources, check=True) == []


@pytest.mark.parametrize("mutation", ["edit", "delete", "stale"])
@pytest.mark.parametrize("extension", ["py", "ts"])
def test_every_generated_source_change_is_detected_and_repaired(tmp_path: Path, mutation: str, extension: str) -> None:
    sources = {"models": {f"item.{extension}": "value = 1\n"}, "_operations": {f"call.{extension}": "value = 2\n"}}
    synchronize(tmp_path, sources, check=False)
    handwritten = tmp_path / "client.py"
    handwritten.write_text("owned by the SDK author")
    changed = tmp_path / "models" / f"item.{extension}"
    if mutation == "edit":
        changed.write_text("broken")
    elif mutation == "delete":
        changed.unlink()
    else:
        changed = tmp_path / "_operations" / f"stale.{extension}"
        changed.write_text("stale")
    before = changed.read_text() if changed.exists() else None
    differences = synchronize(tmp_path, sources, check=True)
    assert len(differences) == 1 and str(changed) in differences[0]
    assert (changed.read_text() if changed.exists() else None) == before
    assert synchronize(tmp_path, sources, check=False) == differences
    assert synchronize(tmp_path, sources, check=True) == []
    assert handwritten.read_text() == "owned by the SDK author"
