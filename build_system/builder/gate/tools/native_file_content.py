"""Assemble and execute the feature-only authenticated Guest FileContent owner."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

from capsem_builder import crypto
from capsem_builder.cache.capacity import RequiredCapacity
from capsem_builder.cache.config import load_paths
from capsem_builder.cache.enforcement import enforce_repository
from capsem_builder.cache.leases import release_path, retain_generation
from capsem_builder.gate.boundedlease import leased
from capsem_builder.gate.config import GateConfig, load
from capsem_builder.gate.qualification import from_environment
from capsem_builder.gate.sourcecommit import source_commit_for_checkout, tree_of
from capsem_builder.release.tools.package_payload import package_payload_files
from capsem_builder.release.tools.test_binary_provenance import (
    COHORT_VARIABLE,
    verify_frozen_binary,
)


def artifacts(log: Path, source: Path, *, tests: bool) -> dict[str, Path]:
    """Select only this successful Cargo invocation's explicit binary artifacts."""
    rows = [json.loads(line) for line in log.read_text().splitlines() if line.startswith("{")]
    completed = [index for index, row in enumerate(rows) if row.get("reason") == "build-finished"]
    if not completed or completed[-1] != len(rows) - 1 or rows[-1].get("success") is not True:
        raise ValueError("native artifact requires a completed successful Cargo producer")
    start = completed[-2] + 1 if len(completed) > 1 else 0
    found = {}
    for row in rows[start:-1]:
        if row.get("reason") != "compiler-artifact" or not row.get("executable"):
            continue
        target = row["target"]
        manifest = Path(row["manifest_path"]).resolve()
        executable = Path(row["executable"])
        if (
            target["kind"] != ["bin"]
            or row["profile"]["test"] is not tests
            or not manifest.is_relative_to(source.resolve())
            or not executable.is_absolute()
            or target["name"] in found
        ):
            raise ValueError("foreign or ambiguous native Cargo generation")
        if tests and (
            manifest != source.resolve() / "crates/capsem-service/Cargo.toml"
            or target["name"] != "capsem-service"
            or "native-acceptance" not in row["features"]
        ):
            raise ValueError("native test artifact lacks its dedicated service feature")
        found[target["name"]] = executable
    if not found:
        raise ValueError("current Cargo invocation produced no native artifacts")
    return found


def source_identity(config: GateConfig) -> dict[str, str]:
    commit = str(source_commit_for_checkout(config.root))
    tree = tree_of(config.root, commit)
    if tree is None:
        raise ValueError("native owner source tree is unavailable")
    digest = subprocess.run(
        [
            sys.executable,
            str(config.path(config.candidate.source_digest_script)),
            "--root",
            str(config.root),
        ],
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    ).stdout.strip()
    if re.fullmatch(r"[0-9a-f]{64}", digest) is None:
        raise ValueError("native owner source state is unavailable")
    return {"source_commit": commit, "source_tree": tree, "source_digest": digest}


def begin(config: GateConfig) -> None:
    settings = config.functional.native_file_content
    target = config.path(settings.source_receipt)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(source_identity(config), sort_keys=True))
    for configured in (settings.cargo_log, settings.host_log):
        config.path(configured).write_text("")


def verify_host_roles(config: GateConfig, bin_dir: Path, package: Path | None) -> None:
    if package is not None:
        payload = package_payload_files(package)
        for name in config.signing.built:
            matches = [data for path, data in payload.items() if Path(path).name == name]
            if len(matches) != 1 or crypto.blake3(matches[0]).hexdigest() != crypto.digest_file(
                bin_dir / name
            ):
                raise ValueError(f"native role differs from selected package: {name}")
    elif bin_dir.resolve() != config.path(config.modules.default_bin_dir).resolve():
        manifest = os.environ.get(COHORT_VARIABLE)
        if not manifest:
            raise ValueError("custom native host directory requires frozen source provenance")
        for name in config.signing.built:
            verify_frozen_binary(bin_dir / name, Path(manifest), config.root)
    else:
        produced = artifacts(
            config.path(config.functional.native_file_content.host_log), config.root, tests=False
        )
        if set(produced) != set(config.signing.built) or any(
            produced[name].resolve() != (bin_dir / name).resolve() for name in produced
        ):
            raise ValueError("native host roles were not produced by the current source build")


def assemble(config: GateConfig, bin_dir: Path, package: Path | None) -> Path:
    settings = config.functional.native_file_content
    qualification = from_environment(config)
    expected_package = (
        None if qualification.package is None else config.path(qualification.package).resolve()
    )
    if (
        bin_dir.resolve() != config.path(qualification.bin_dir).resolve()
        or (None if package is None else package.resolve()) != expected_package
    ):
        raise ValueError("native roles must use the plan-selected artifact pairing")
    source = json.loads(config.path(settings.source_receipt).read_text())
    if source != source_identity(config):
        raise ValueError("source changed while compiling the native owner")
    verify_host_roles(config, bin_dir, package)
    artifact = artifacts(config.path(settings.cargo_log), config.root, tests=True)["capsem-service"]
    sources = {name: bin_dir / name for name in config.signing.built}
    sources[settings.artifact_name] = artifact
    binaries = {}
    for name, path in sources.items():
        if path.is_symlink() or not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError(f"native owner lacks a real executable role: {name}: {path}")
        binaries[name] = crypto.digest_file(path)
    receipt = {
        "schema": "capsem.native-file-content.v1",
        **source,
        "binaries": binaries,
        "cohort_package": None if package is None else crypto.digest_file(package),
    }
    key = (
        settings.generation_prefix
        + crypto.blake3(json.dumps(receipt, sort_keys=True).encode()).hexdigest()
    )
    compiler_logs = {}
    for configured in (settings.cargo_log, settings.host_log):
        log = config.path(configured)
        if log.stat().st_size:
            if log.name in sources or log.name == settings.receipt_name:
                raise ValueError("native compiler log collides with an artifact role")
            sources[log.name] = log
            compiler_logs[log.name] = crypto.digest_file(log)
    paths = load_paths(config.root)
    stage = paths.policy.stages[settings.cache_stage]
    if stage.lease_template is None:
        raise ValueError("native acceptance artifact stage must protect generation leases")
    lease = paths.stage(settings.cache_stage) / stage.lease_template.format(key=key)
    retain_generation(paths, settings.cache_stage, key)
    try:
        directory = paths.stage(settings.cache_stage) / key
        if directory.is_symlink() or (directory.exists() and not directory.is_dir()):
            raise ValueError("native artifact generation is not a real directory")
        hashes = {**binaries, **compiler_logs}
        missing = {}
        incoming = 0
        for name, path in sources.items():
            target = directory / name
            if target.is_symlink() or (target.exists() and not target.is_file()):
                raise ValueError(f"native artifact role is not a real file: {target}")
            present = target.exists()
            equal = present and crypto.digest_file(target) == hashes[name]
            if present and name in binaries and not equal:
                raise ValueError(f"native artifact generation changed: {target}")
            if not equal:
                size = path.stat().st_size
                missing[name] = size
                # copy2 truncates the destination in place: no second/temp copy.
                incoming += max(0, size - (target.stat().st_size if present else 0))
        encoded = json.dumps(receipt, sort_keys=True).encode()
        receipt_path = directory / settings.receipt_name
        if receipt_path.is_symlink() or (
            receipt_path.exists()
            and (not receipt_path.is_file() or receipt_path.read_bytes() != encoded)
        ):
            raise ValueError("native artifact receipt changed")
        receipt_missing = not receipt_path.exists()
        incoming += len(encoded) if receipt_missing else 0
        admitted = enforce_repository(
            paths,
            paths.policy,
            settings.cache_stage,
            reason="native FileContent artifact needs original binaries, receipt and compiler logs",
            required=RequiredCapacity(bytes=incoming, entries=0 if directory.exists() else 1),
        )
        if admitted.violations:
            raise ValueError(
                "native artifact capacity unavailable: " + "; ".join(admitted.violations)
            )
        directory.mkdir(exist_ok=True)
        for name, size in missing.items():
            path, target = sources[name], directory / name
            if path.stat().st_size != size:
                raise ValueError("native artifact source changed after capacity admission")
            shutil.copy2(path, target)
            if target.is_symlink() or crypto.digest_file(target) != hashes[name]:
                raise ValueError(f"native artifact generation changed: {target}")
        if source != source_identity(config):
            raise ValueError("source changed while freezing the native owner")
        if receipt_missing:
            receipt_path.write_bytes(encoded)
        selection = config.path(settings.selection)
        selection.parent.mkdir(parents=True, exist_ok=True)
        selection.write_text(
            json.dumps(
                {"key": key, "source": source, "compiler_logs": compiler_logs}, sort_keys=True
            )
        )
        print(
            f"Native FileContent owner: {directory}; selected host roles: {len(config.signing.built)}",
            flush=True,
        )
        return directory
    finally:
        release_path(lease)


def selected(config: GateConfig) -> tuple[Path, str]:
    settings = config.functional.native_file_content
    selection = json.loads(config.path(settings.selection).read_text())
    key = selection["key"]
    if (
        not isinstance(key, str)
        or not key.startswith(settings.generation_prefix)
        or re.fullmatch(r"[0-9a-f]{64}", key[len(settings.generation_prefix) :]) is None
        or selection["source"] != source_identity(config)
    ):
        raise ValueError("native acceptance selection is foreign to this source")
    return load_paths(config.root).stage(settings.cache_stage) / key, key


def main(root: Path) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("begin", "assemble"))
    parser.add_argument("--bin-dir", type=Path)
    parser.add_argument("--package", type=Path)
    args = parser.parse_args()
    config = load(root)
    if args.mode == "begin":
        begin(config)
    elif args.bin_dir is None:
        parser.error("assembly requires the plan-selected --bin-dir")
    else:
        directory = args.bin_dir if args.bin_dir.is_absolute() else config.path(str(args.bin_dir))
        with leased(
            (sys.executable, str(Path(__file__)), "assemble"), root, os.environ, force=True
        ):
            assemble(config, directory, args.package)
    return 0
