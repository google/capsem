"""Run with an isolated installed SDK; no workspace test imports are allowed."""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import importlib.metadata
import json
import sys
import tarfile
import threading
import tomllib
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import aiohttp
import capsem
import pydantic
import yarl
from capsem import Hypervisor, Registry, models


def archive_payload(archive: Path) -> dict[str, bytes]:
    if archive.suffix == ".whl":
        with zipfile.ZipFile(archive) as stream:
            return {
                name: stream.read(name)
                for name in stream.namelist()
                if name.startswith("capsem/") and not name.endswith("/")
            }
    with tarfile.open(archive) as stream:
        result = {}
        for member in stream.getmembers():
            parts = Path(member.name).parts[1:]
            if parts and parts[0] == "capsem" and member.isfile():
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
    for module in [capsem, aiohttp, pydantic, yarl]:
        origin = Path(module.__file__).resolve()
        assert origin.is_relative_to(prefix), (module.__name__, origin)
        origins[module.__name__] = str(origin)
    distribution = importlib.metadata.distribution("capsem")
    with (source / "sdk/python/pyproject.toml").open("rb") as file:
        manifest = tomllib.load(file)["project"]
    assert distribution.version == manifest["version"]
    assert distribution.metadata["Requires-Python"] == manifest["requires-python"]
    installed = {
        dist.metadata["Name"].lower() for dist in importlib.metadata.distributions()
    }
    assert not installed.intersection(
        {"pytest", "build", "hatchling", "editables", "ruff", "ty"}
    )
    payload = archive_payload(args.archive)
    assert "capsem/py.typed" in payload
    assert "capsem/_images.py" in payload
    base = Path(capsem.__file__).parent.parent
    actual = {
        str(path.relative_to(base))
        for path in (base / "capsem").rglob("*")
        if path.is_file() and "__pycache__" not in path.parts
    }
    assert actual == set(payload)
    for relative, content in payload.items():
        assert not (base / relative).is_symlink()
        assert (base / relative).read_bytes() == content
        assert (source / "sdk/python" / relative).read_bytes() == content
    assert "editable" not in (distribution.read_text("direct_url.json") or "").lower()

    pin = "registry.example/code@sha256:" + "a" * 64
    received = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:
            pass

        def do_GET(self) -> None:
            self.reply(
                {
                    "images": [
                        {
                            "name": "code",
                            "description": "Tools",
                            "architectures": ["amd64"],
                            "image": pin,
                            "cached": "unknown",
                        }
                    ]
                }
            )

        def do_POST(self) -> None:
            self.reply(
                {"image": "code", "resolved": pin, "digest": "sha256:" + "b" * 64}
            )

        def reply(self, body: object) -> None:
            assert self.headers.get("Authorization") == "Bearer fixture-token"
            raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            received.append((self.command, self.path, json.loads(raw) if raw else None))
            data = json.dumps(body).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    async def probe(url: str) -> None:
        async with Hypervisor(url, "fixture-token") as hv:
            catalog = await hv.images.list(refresh=True)
            assert isinstance(catalog, models.ImageListResponse)
            assert catalog.images[0].image == pin
            assert catalog.images[0].cached == models.ImageCacheState.UNKNOWN
            result = await hv.images.pull(
                "code", registry=Registry(username="fixture", password="fixture-access")
            )
            assert result.resolved == pin
            await hv.images.pull("code")
            images = hv.images
        try:
            await images.list()
        except RuntimeError as error:
            assert "closed" in str(error)
        else:
            raise AssertionError("closed parent left an operational image handle")

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        asyncio.run(probe(f"http://127.0.0.1:{server.server_port}"))
    finally:
        server.shutdown()
        server.server_close()
        thread.join(2)
    assert [(method, path) for method, path, _ in received] == [
        ("GET", "/images?refresh=true"),
        ("POST", "/images/pull"),
        ("POST", "/images/pull"),
    ]
    assert received[1][2] == {
        "image": "code",
        "registry": {
            "username": "fixture",
            "password": "fixture-access",
            "ca_pem": None,
        },
    }
    assert received[2][2] == {"image": "code"}
    report = {
        "archive": str(args.archive),
        "sha256": hashlib.sha256(args.archive.read_bytes()).hexdigest(),
        "version": distribution.version,
        "requires": distribution.requires,
        "prefix": str(prefix),
        "origins": origins,
        "payload_files": len(payload),
        "http_paths": [path for _, path, _ in received],
        "isolated": True,
        "ok": True,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print("SDK_IMAGE_PACKAGE_ACCEPTANCE_OK")


if __name__ == "__main__":
    main()
