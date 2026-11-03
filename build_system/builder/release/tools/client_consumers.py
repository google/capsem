"""Install published clients anonymously and use their real local gateway APIs.

This post-publication observation cannot run before the immutable versions
exist. Pre-publication qualification exercises the same packaged clients in
the complete SDK, Inspect and packed MCP acceptance suites.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

from ...gate.config import GateConfig
from . import client_consumer_probes as probes


def clean_environment(environment: dict[str, str]) -> dict[str, str]:
    forbidden = {
        "PYPI_TOKEN", "NPM_TOKEN", "CARGO_REGISTRY_TOKEN", "NODE_AUTH_TOKEN", "UV_PUBLISH_TOKEN",
        "PYTHONPATH", "PYTHONHOME", "VIRTUAL_ENV", "NODE_PATH", "NODE_OPTIONS", "GH_TOKEN", "GITHUB_TOKEN",
        "CAPSEM_GATEWAY_URL", "CAPSEM_GATEWAY_TOKEN", "NPM_CONFIG_USERCONFIG", "npm_config_userconfig",
        "ACTIONS_ID_TOKEN_REQUEST_URL", "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
    }
    return {key: value for key, value in environment.items()
            if key not in forbidden and not key.startswith(("UV_", "PIP_", "NPM_", "npm_", "CARGO_REGISTRIES_"))}


def verify(config: GateConfig, environment: dict[str, str]) -> None:
    version = tomllib.loads(config.path("Cargo.toml").read_text())["workspace"]["package"]["version"]
    extension = tomllib.loads(config.path(config.integrations_inspect_ai.manifest).read_text())["project"]["version"]
    with tempfile.TemporaryDirectory(prefix="capsem-public-clients-") as directory:
        work = Path(directory)
        if work.is_relative_to(config.root.resolve()):
            raise ValueError("public consumers must be outside the checkout")
        child = clean_environment(environment)
        home = work / "home"
        home.mkdir()
        (home / ".npmrc").write_text("")
        child.update(HOME=str(home), NPM_CONFIG_USERCONFIG=str(home / ".npmrc"),
                     CLIENT_VERSION=version, INSPECT_VERSION=extension)

        def run(argv: list[str], *, timeout: int = 300) -> None:
            subprocess.run(argv, cwd=work, env=child, check=True, timeout=timeout)

        python = work / "python/bin/python"
        run(["uv", "venv", "--python", sys.executable, str(python.parent.parent)])
        run(["uv", "pip", "install", "--no-cache", "--index-url", "https://pypi.org/simple",
             "--python", str(python), f"capsem=={version}", f"inspect-capsem-sandbox=={extension}"])
        run(["uv", "pip", "check", "--python", str(python)])
        (work / "probe.py").write_text(probes.PYTHON)
        run([str(python), "-I", str(work / "probe.py")])

        (work / "package.json").write_text(json.dumps({"name": "public-capsem-consumer", "private": True}))
        run(["npm", "install", "--registry=https://registry.npmjs.org", "--cache", str(work / "npm-cache"),
             "--ignore-scripts", "--no-audit", "--no-fund", f"@capsem/sdk@{version}", f"@capsem/mcp@{version}"])
        lock = json.loads((work / "package-lock.json").read_text())["packages"]
        for name in ("sdk", "mcp"):
            row = lock[f"node_modules/@capsem/{name}"]
            if row.get("version") != version or not row.get("resolved", "").startswith("https://registry.npmjs.org/") or row.get("link"):
                raise ValueError("npm consumer resolved a non-registry Capsem package")
        (work / "probe.mjs").write_text(probes.NODE)
        run(["node", "probe.mjs"])

        print("PUBLIC_CLIENT_COHORT_OK", flush=True)
