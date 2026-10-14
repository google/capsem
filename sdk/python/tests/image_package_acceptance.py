"""Run with an isolated installed SDK; no workspace test imports are allowed."""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import importlib.metadata
import json
import re
import sys
import tarfile
import threading
import time
import tomllib
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import aiohttp
import capsem
import pydantic
import yarl
from capsem import VM, Hypervisor, Registry, models


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


def normalized_requirement(requirement: str) -> tuple[str, tuple[str, ...]]:
    match = re.fullmatch(r"([A-Za-z0-9._-]+)(.*)", requirement.replace(" ", ""))
    assert match is not None
    return match[1].lower(), tuple(sorted(filter(None, match[2].split(","))))


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
    assert {
        normalized_requirement(requirement) for requirement in distribution.requires or []
    } == {normalized_requirement(requirement) for requirement in manifest["dependencies"]}
    installed = {dist.metadata["Name"].lower() for dist in importlib.metadata.distributions()}
    assert not installed.intersection({"pytest", "build", "hatchling", "editables", "ruff", "ty"})
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
    secret, reference = "fixture-injected-secret", "credential:blake3:" + "c" * 64
    received = []
    restore_entered = threading.Event()
    provision = {
        "id": "restore-vm",
        "name": "restore",
        "status": "Running",
        "available_actions": [],
    }

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:
            pass

        def do_GET(self) -> None:
            if self.path in {"/vms/restore-vm/info", "/vms/slow/info"}:
                self.reply({**provision, "pid": 1})
                return
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
            if self.path == "/credentials/inject":
                self.reply({"credential_ref": reference})
                return
            if self.path in {"/vms/restore-vm/start", "/vms/restore-vm/resume"}:
                self.reply(provision)
                return
            self.reply({"image": "code", "resolved": pin, "digest": "sha256:" + "b" * 64})

        def reply(self, body: object) -> None:
            assert self.headers.get("Authorization") == "Bearer fixture-token"
            raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            request = json.loads(raw) if raw else None
            received.append((self.command, self.path, request))
            if self.path == "/credentials/inject":
                assert isinstance(body, dict) and isinstance(request, dict)
                body = {**body, "storage": request["storage"]}
            if self.path.endswith(("/start", "/resume")):
                restore_entered.set()
                time.sleep(0.3)
            elif self.path == "/vms/slow/info":
                time.sleep(0.3)
            data = json.dumps(body).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            try:
                self.wfile.write(data)
            except (BrokenPipeError, ConnectionResetError):
                # Cancellation/default read timeout deliberately closes these sockets.
                assert self.path.startswith("/vms/")

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
            for storage in ("memory", "file"):
                injected = await hv.credentials.inject("openai", secret, storage=storage)
                assert injected.credential_ref == reference and injected.storage == storage
                assert secret not in repr(injected)
            private = models.CredentialInjectRequest(
                provider=models.CredentialInjectProvider.OPENAI, value=secret
            )
            assert secret not in repr(private)
            try:
                models.CredentialInjectRequest.model_validate({"provider": secret, "value": secret})
            except pydantic.ValidationError as error:
                assert secret not in str(error) and secret not in repr(error.errors())
            else:
                raise AssertionError("invalid installed private input was accepted")
            images = hv.images
        try:
            await images.list()
        except RuntimeError as error:
            assert "closed" in str(error)
        else:
            raise AssertionError("closed parent left an operational image handle")

        async with VM(url, "fixture-token", id="restore-vm", timeout=0.05) as vm:
            for operation in (vm.start, vm.resume):
                result = await operation()
                assert isinstance(result, models.ProvisionResponse)
                assert (
                    result.id == "restore-vm" and result.status == models.VmLifecycleState.RUNNING
                )
            for operation in (vm.start, vm.resume):
                restore_entered.clear()
                pending = asyncio.create_task(operation())
                try:
                    assert await asyncio.to_thread(restore_entered.wait, 2), (
                        "restore did not reach HTTP fixture"
                    )
                    pending.cancel()
                    try:
                        await pending
                    except asyncio.CancelledError:
                        pass
                    else:
                        raise AssertionError("cancelled restore completed successfully")
                    info = await vm.info()
                    assert info.id == "restore-vm" and info.pid == 1
                finally:
                    if not pending.done():
                        pending.cancel()
                        await asyncio.gather(pending, return_exceptions=True)
        async with VM(url, "fixture-token", id="slow", timeout=0.05) as vm:
            try:
                await vm.info()
            except TimeoutError:
                pass
            else:
                raise AssertionError("ordinary read lost the shorter transport deadline")

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        asyncio.run(probe(f"http://127.0.0.1:{server.server_port}"))
    finally:
        server.shutdown()
        server.server_close()
        thread.join(2)
    assert not thread.is_alive(), "HTTP fixture did not stop"
    image_requests = [request for request in received if request[1].startswith("/images")]
    lifecycle_requests = [request for request in received if request[1].startswith("/vms/")]
    credential_requests = [request for request in received if request[1] == "/credentials/inject"]
    assert len(image_requests) + len(lifecycle_requests) + len(credential_requests) == len(received)
    assert credential_requests == [
        ("POST", "/credentials/inject", {"provider": "openai", "value": secret, "storage": storage})
        for storage in ("memory", "file")
    ]
    assert [(method, path) for method, path, _ in image_requests] == [
        ("GET", "/images?refresh=true"),
        ("POST", "/images/pull"),
        ("POST", "/images/pull"),
    ]
    assert lifecycle_requests == [
        ("POST", "/vms/restore-vm/start", None),
        ("POST", "/vms/restore-vm/resume", None),
        ("POST", "/vms/restore-vm/start", None),
        ("GET", "/vms/restore-vm/info", None),
        ("POST", "/vms/restore-vm/resume", None),
        ("GET", "/vms/restore-vm/info", None),
        ("GET", "/vms/slow/info", None),
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
        "python_version": list(sys.version_info[:3]),
        "executable": sys.executable,
        "origins": origins,
        "payload_files": len(payload),
        "http_paths": [path for _, path, _ in image_requests],
        "lifecycle_paths": [path for _, path, _ in lifecycle_requests],
        "credential_paths": [path for _, path, _ in credential_requests],
        "private_input_redacted": True,
        "isolated": True,
        "ok": True,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print("SDK_IMAGE_PACKAGE_ACCEPTANCE_OK")


if __name__ == "__main__":
    main()
