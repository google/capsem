"""Run with an isolated installed inspect-capsem-sandbox; no workspace imports are allowed."""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import sys
import tarfile
import tomllib
import zipfile
from pathlib import Path

import capsem
import inspect_ai
import inspect_capsem
import pydantic
import yaml
from inspect_ai.util._sandbox.registry import registry_find_sandboxenv
from inspect_capsem import CapsemSandboxEnvironment


def archive_payload(archive: Path) -> dict[str, bytes]:
    if archive.suffix == ".whl":
        with zipfile.ZipFile(archive) as stream:
            return {
                name: stream.read(name)
                for name in stream.namelist()
                if name.startswith("inspect_capsem/") and not name.endswith("/")
            }
    with tarfile.open(archive) as stream:
        result = {}
        for member in stream.getmembers():
            parts = Path(member.name).parts[1:]
            if parts and parts[0] == "inspect_capsem" and member.isfile():
                file = stream.extractfile(member)
                assert file is not None
                result[str(Path(*parts))] = file.read()
        return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    assert sys.flags.isolated, "use Python -I in a clean non-editable environment"
    prefix = Path(sys.prefix).resolve()
    source = args.source_root.resolve()
    assert not prefix.is_relative_to(source)
    assert all(not Path(path).resolve().is_relative_to(source) for path in sys.path)
    origins = {}
    for module in [inspect_capsem, capsem, inspect_ai, pydantic, yaml]:
        mod_file = module.__file__
        assert mod_file is not None
        origin = Path(mod_file).resolve()
        assert origin.is_relative_to(prefix), (module.__name__, origin)
        origins[module.__name__] = str(origin)

    distribution = importlib.metadata.distribution("inspect-capsem-sandbox")
    sdk_dist = importlib.metadata.distribution("capsem")
    with (source / "integrations/inspect-ai/pyproject.toml").open("rb") as file:
        manifest = tomllib.load(file)["project"]
    assert distribution.version == manifest["version"]
    assert distribution.metadata["Requires-Python"] == manifest["requires-python"]
    assert "editable" not in (distribution.read_text("direct_url.json") or "").lower()
    assert "editable" not in (sdk_dist.read_text("direct_url.json") or "").lower()

    installed = {dist.metadata["Name"].lower() for dist in importlib.metadata.distributions()}
    assert not installed.intersection({"pytest", "build", "hatchling", "editables", "ruff", "ty"})

    eps = [ep for ep in importlib.metadata.entry_points(group="inspect_ai") if ep.name == "capsem"]
    assert len(eps) == 1
    assert eps[0].value == "inspect_capsem._registry"
    assert eps[0].dist is not None and eps[0].dist.name == "inspect-capsem-sandbox"
    loaded_mod = eps[0].load()
    assert loaded_mod.capsem_sandbox_environment is CapsemSandboxEnvironment
    assert registry_find_sandboxenv("capsem") is CapsemSandboxEnvironment

    payload = archive_payload(args.archive)
    assert "inspect_capsem/py.typed" in payload
    assert "inspect_capsem/sandbox.py" in payload
    mod_file = inspect_capsem.__file__
    assert mod_file is not None
    base = Path(mod_file).parent.parent
    actual = {
        str(path.relative_to(base))
        for path in (base / "inspect_capsem").rglob("*")
        if path.is_file() and "__pycache__" not in path.parts
    }
    assert actual == set(payload)
    for relative, content in payload.items():
        assert not (base / relative).is_symlink()
        assert (base / relative).read_bytes() == content
        assert (source / "integrations/inspect-ai" / relative).read_bytes() == content

    report = {
        "archive": str(args.archive),
        "sha256": hashlib.sha256(args.archive.read_bytes()).hexdigest(),
        "version": distribution.version,
        "requires": distribution.requires,
        "prefix": str(prefix),
        "origins": origins,
        "payload_files": len(payload),
        "entry_point": eps[0].value,
        "isolated": True,
        "ok": True,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print("SDK_IMAGE_PACKAGE_ACCEPTANCE_OK")


if __name__ == "__main__":
    main()
