"""Publish verified client archives only after their exact native release passed."""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import tempfile
import time
import tomllib
from collections.abc import Mapping
from pathlib import Path

from ...gate.config import GateConfig, load
from .client_package_archives import Package, collect
from .client_registry import inspect

CREDENTIALS = ("PYPI_TOKEN",)


def require_credentials(environment: Mapping[str, str]) -> None:
    missing = [name for name in CREDENTIALS if not environment.get(name, "").strip()]
    if missing:
        raise ValueError("missing registry publisher secrets: " + ", ".join(missing))


def require_source_context(
    source: str, environment: Mapping[str, str], head: str, tag: str, dirty: bool
) -> None:
    expected = {
        "GITHUB_REPOSITORY": "google/capsem",
        "GITHUB_SHA": source,
        "GITHUB_REF": f"refs/tags/capsem-source-{source}",
    }
    if not re.fullmatch(r"[0-9a-f]{40}", source) or any(environment.get(key) != value for key, value in expected.items()):
        raise ValueError("client publication requires the exact immutable native release context")
    if head != source or tag != source or dirty:
        raise ValueError("client publication checkout/tag differs from the clean native source")


def validate_order(packages: tuple[Package, ...]) -> None:
    ready: dict[tuple[str, str], str] = {}
    for package in packages:
        key = (package.registry, package.name)
        if key in ready:
            raise ValueError("duplicate client package identity")
        for name, version in package.dependencies:
            if ready.get((package.registry, name)) != version:
                raise ValueError(f"client dependency unavailable before publication: {package.name} -> {name}")
        ready[key] = package.version


def _read_only_environment(environment: Mapping[str, str]) -> dict[str, str]:
    sensitive = {*CREDENTIALS, "NPM_TOKEN", "CARGO_REGISTRY_TOKEN", "NODE_AUTH_TOKEN", "UV_PUBLISH_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"}
    return {key: value for key, value in environment.items() if key not in sensitive and not key.lower().startswith("npm_")}


def _run(config: GateConfig, argv: list[str], environment: Mapping[str, str]) -> None:
    subprocess.run(argv, cwd=config.root, env=dict(environment), check=True)


def _upload(config: GateConfig, package: Package, missing: list[Path], environment: Mapping[str, str]) -> None:
    child = _read_only_environment(environment)
    if package.registry == "pypi":
        child["UV_PUBLISH_TOKEN"] = environment["PYPI_TOKEN"]
        _run(config, ["uv", "publish", "--publish-url", "https://upload.pypi.org/legacy/",
                      "--trusted-publishing", "never", *map(str, missing)], child)
    elif package.registry == "npm":
        # npm uses the workflow's short-lived OIDC authority, never a registry
        # token. whoami cannot validate trusted-publisher permission.
        if environment.get("GITHUB_ACTIONS") != "true" or any(
            not environment.get(key, "").strip()
            for key in ("ACTIONS_ID_TOKEN_REQUEST_URL", "ACTIONS_ID_TOKEN_REQUEST_TOKEN")
        ):
            raise ValueError("npm publication requires GitHub Actions OIDC authority")
        # setup-node's legacy .npmrc and ambient npm configuration cannot
        # supply registry credentials or override this OIDC-only publication.
        with tempfile.TemporaryDirectory(prefix="capsem-npm-publisher-") as directory:
            for kind in ("USER", "GLOBAL"):
                path = Path(directory) / f"{kind.lower()}.npmrc"
                path.write_text("")
                child[f"NPM_CONFIG_{kind}CONFIG"] = str(path)
            _run(config, ["npm", "publish", str(package.archives[0]), "--access", "public",
                          "--registry=https://registry.npmjs.org", "--ignore-scripts", "--provenance",
                          "--tag", "latest"], child)
    else:
        raise ValueError(f"registry publication is disabled: {package.registry}")


def _await_public(package: Package) -> None:
    deadline = time.monotonic() + 120
    while True:
        if set(inspect(package)) == set(package.archives):
            return
        if time.monotonic() >= deadline:
            raise ValueError(f"registry did not expose all verified package bytes: {package.name}")
        time.sleep(2)


def publish(config: GateConfig, packages: tuple[Package, ...], environment: Mapping[str, str]) -> None:
    require_credentials(environment)
    # Cargo remains locally qualified; registry publication is deferred in
    # https://github.com/google/capsem/issues/382 until its CI setup is ready.
    packages = tuple(package for package in packages if package.registry != "crates")
    validate_order(packages)
    # All immutable conflicts are checked before the first upload. A failure
    # resumes only exact existing bytes; there is no skip-existing escape hatch.
    existing = {package: inspect(package) for package in packages}
    for package in packages:
        missing = [archive for archive in package.archives if archive not in existing[package]]
        if missing:
            _upload(config, package, missing, environment)
        _await_public(package)
        print(f"Public verified: {package.registry} {package.name} {package.version}", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    commands.add_parser("check-credentials")
    publication = commands.add_parser("publish")
    publication.add_argument("--source-commit", required=True)
    publication.add_argument("--release-tag", required=True)
    publication.add_argument("--original-run-id", default="")
    arguments = parser.parse_args()
    if arguments.action == "check-credentials":
        require_credentials(os.environ)
        print("Required registry secrets are present; publication verifies registry permissions.")
        return
    config = load(Path.cwd())
    def capture(*argv: str) -> str:
        return subprocess.check_output(argv, cwd=config.root, text=True).strip()
    version = tomllib.loads(config.path("Cargo.toml").read_text())["workspace"]["package"]["version"]
    if arguments.release_tag != f"v{version}":
        raise ValueError("client cohort differs from selected native release version")
    source = arguments.source_commit
    context_tag = f"capsem-source-{source}" if arguments.original_run_id else arguments.release_tag
    require_source_context(source, os.environ, capture("git", "rev-parse", "HEAD"),
                           capture("git", "rev-parse", f"{context_tag}^{{commit}}"),
                           bool(capture("git", "status", "--porcelain")))
    if arguments.original_run_id:
        from .client_recovery import original_run, verify_archives
        source = original_run(config.root, arguments.original_run_id, arguments.release_tag)
        packages = collect(config, source)
        verify_archives(config, packages, source)
    else:
        packages = collect(config, source)
    publish(config, packages, os.environ)


if __name__ == "__main__":
    main()
