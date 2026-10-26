"""Dependency declarations and resolved lockfile footprint, without executing package managers."""

from __future__ import annotations

import fnmatch
import json
import re
import tomllib
from pathlib import Path

import yaml
from packaging.requirements import Requirement
from packaging.utils import canonicalize_name


def rust_members(root):
    file = root / "Cargo.toml"
    if not file.exists():
        return {}, {}
    document = tomllib.loads(file.read_text())
    workspace = document.get("workspace", {})
    result = {}
    if "package" in document:
        result["."] = document
    for pattern in workspace.get("members", []):
        for candidate in root.glob(pattern):
            manifest = candidate / "Cargo.toml"
            if manifest.exists() and not any(
                fnmatch.fnmatch(candidate.relative_to(root).as_posix(), p)
                for p in workspace.get("exclude", [])
            ):
                item = tomllib.loads(manifest.read_text())
                if "package" in item:
                    result[candidate.relative_to(root).as_posix()] = item
    return document, result


def declarations(root, paths):
    rootdoc, members = rust_members(root)
    inherited = rootdoc.get("workspace", {}).get("dependencies", {})
    member_names = {doc["package"]["name"] for doc in members.values()}
    result = []

    def add(ecosystem, workspace, kind, name, spec, internal=False, target=""):
        result.append(
            {
                "ecosystem": ecosystem,
                "workspace": workspace,
                "kind": kind,
                "package": name,
                "internal": int(internal),
                "target_condition": target,
                "constraint": str(spec),
            }
        )

    for doc in members.values():
        scopes = [("", doc), *list(doc.get("target", {}).items())]
        for target, scope in scopes:
            for section, kind in [
                ("dependencies", "runtime"),
                ("dev-dependencies", "development"),
                ("build-dependencies", "build"),
            ]:
                for alias, spec in scope.get(section, {}).items():
                    if isinstance(spec, dict) and spec.get("workspace"):
                        spec = inherited[alias]
                    name = spec.get("package", alias) if isinstance(spec, dict) else alias
                    add(
                        "rust",
                        doc["package"]["name"],
                        kind,
                        name,
                        spec,
                        name in member_names,
                        target,
                    )
    npm_manifests = []
    python_manifests = []
    for path in paths:
        if set(Path(path).parts) & {"tests", "fixtures", "fuzz"}:
            continue
        if Path(path).name == "package.json":
            npm_manifests.append((path, json.loads((root / path).read_text())))
        if Path(path).name == "pyproject.toml":
            python_manifests.append((path, tomllib.loads((root / path).read_text())))
    local_npm = {d.get("name") for _, d in npm_manifests}
    for path, doc in npm_manifests:
        for section, kind in [
            ("dependencies", "runtime"),
            ("devDependencies", "development"),
            ("optionalDependencies", "optional_runtime"),
            ("peerDependencies", "peer"),
        ]:
            for name, spec in doc.get(section, {}).items():
                internal = name in local_npm or str(spec).startswith(
                    ("link:", "workspace:", "file:")
                )
                add("npm", str(Path(path).parent), kind, name, spec, internal)
    local_python = {
        canonicalize_name(d.get("project", {}).get("name", "")) for _, d in python_manifests
    }
    for path, doc in python_manifests:
        project = doc.get("project", {})
        groups = [
            ("runtime", project.get("dependencies", [])),
            ("build", doc.get("build-system", {}).get("requires", [])),
        ]
        groups += [
            ("optional_runtime", group)
            for group in project.get("optional-dependencies", {}).values()
        ]
        groups += [("development", group) for group in doc.get("dependency-groups", {}).values()]
        for kind, group in groups:
            for spec in group:
                if isinstance(spec, str):
                    name = canonicalize_name(Requirement(spec).name)
                    add("python", str(Path(path).parent), kind, name, spec, name in local_python)
    return result, members, npm_manifests, python_manifests


def lock_packages(root, paths, ecosystem):
    identities = set()
    lockfiles = []

    def add(name, version):
        if name and version:
            identities.add(
                (canonicalize_name(name) if ecosystem == "python" else name, str(version))
            )

    def npm_key(key):
        key = key.lstrip("/")
        # pnpm <= 5 used /name/version, newer versions use name@version.
        match = re.match(r"^(@[^/]+/[^/@]+|[^/@]+)(?:@|/)([^(/]+)", key)
        return match.groups() if match else (None, None)

    for path in paths:
        name = Path(path).name
        if set(Path(path).parts) & {"tests", "fixtures", "fuzz"}:
            continue
        if ecosystem == "rust" and path == "Cargo.lock":
            lockfiles.append(path)
            for package in tomllib.loads((root / path).read_text()).get("package", []):
                if package.get("source"):
                    add(package["name"], package["version"])
        elif ecosystem == "python" and name == "uv.lock":
            lockfiles.append(path)
            for package in tomllib.loads((root / path).read_text()).get("package", []):
                source = package.get("source", {})
                if not any(k in source for k in ["editable", "virtual", "directory"]):
                    add(package["name"], package["version"])
        elif ecosystem == "npm" and name == "pnpm-lock.yaml":
            lockfiles.append(path)
            document = yaml.safe_load((root / path).read_text())
            for key in document.get("packages", {}):
                add(*npm_key(key))
        elif ecosystem == "npm" and name == "package-lock.json":
            lockfiles.append(path)
            document = json.loads((root / path).read_text())
            if "packages" in document:
                for key, package in document["packages"].items():
                    if "node_modules/" in key and not package.get("link"):
                        add(key.rsplit("node_modules/", 1)[1], package.get("version"))
            else:

                def walk(dependencies):
                    for key, package in dependencies.items():
                        add(key, package.get("version"))
                        walk(package.get("dependencies", {}))

                walk(document.get("dependencies", {}))
    return identities, lockfiles
