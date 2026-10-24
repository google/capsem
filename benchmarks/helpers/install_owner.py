"""Thin reuse of the existing disposable installed owners; no host install."""

from __future__ import annotations

import argparse
import importlib.util
import shutil
import sys
from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.gate.cancellation import unwind_sigterm
from capsem_builder.gate.content import RuntimeContent, SelectedInstallContent
from capsem_builder.gate.install import InstallGate
from capsem_builder.gate.installpairing import NativePairing
from capsem_builder.gate.proc import Runner
from capsem_builder.gate.sourcecommit import SourceCommit, source_commit_for_checkout

from .install_inputs import Prepared, load_inputs, read_bytes, read_object

ROOT = Path(__file__).resolve().parents[2]


def linux_pairing(prepared: Prepared) -> NativePairing:
    inputs = prepared.inputs
    return NativePairing(
        channel=inputs.channel,
        before_manifest=inputs.before.manifest,
        after_manifest=inputs.after.manifest,
        before_package=inputs.before.package,
        before_inputs=inputs.before.runtime_inputs,
        after_inputs=inputs.after.runtime_inputs,
    )


def tart_arguments(prepared: Prepared, work: Path) -> list[str]:
    inputs = prepared.inputs
    tart = inputs.tart
    if tart is None:
        raise ValueError("Tart inputs are absent")
    return [
        "--package",
        str(inputs.after.package),
        "--version",
        prepared.pair["after"]["artifact"]["version"],
        "--manifest-url",
        tart.manifest_url,
        "--manifest-file",
        str(inputs.after.manifest),
        "--sbom",
        str(inputs.after.sbom),
        "--asset-share",
        str(tart.asset_share),
        "--channel",
        inputs.channel,
        "--work-dir",
        str(work),
        "--before-package",
        str(inputs.before.package),
        "--before-manifest-file",
        str(inputs.before.manifest),
        "--before-sbom",
        str(inputs.before.sbom),
        "--tampered-manifest-file",
        str(tart.tampered_manifest),
        "--updated-manifest-file",
        str(tart.updated_manifest),
        "--incompatible-manifest-file",
        str(tart.incompatible_manifest),
    ]


def invoke(root: Path, prepared: Prepared, work: Path, runner: Runner) -> Path:
    if work.exists():
        raise ValueError("Install owner refuses existing output; every observation must be fresh")
    config = gate_config.for_root(root)
    inputs = prepared.inputs
    content = SelectedInstallContent(RuntimeContent.staged(config, inputs.content_root))
    content.require_complete(config, arches=(config.host_arch(),))
    if content.inputs(config).resolve() != inputs.after.runtime_inputs.resolve():
        raise ValueError("Install content and exact after-runtime inputs are different owners")
    if inputs.platform == "linux":
        gate = InstallGate(
            runner,
            content=content,
            source_commit=SourceCommit(inputs.source_commit),
            glowup_pairing=linux_pairing(prepared),
        )
        if gate.package.resolve() != inputs.after.package.resolve():
            raise ValueError(
                "Install after-package is not the existing canonical package producer output"
            )
        gate.run()
        source = config.path(config.install.layout.glowup_evidence)
        # Keep exact raw native reports beside this observation before the
        # existing owner clears its canonical scratch directory on a repeat.
        work.mkdir(parents=True, exist_ok=False)
        for name in (
            "report.json",
            "candidate-apply.json",
            "candidate-apply-request.json",
            "candidate-after-installed.json",
            "candidate-after-doctor.json",
            "candidate-after-winterfell.json",
            "candidate-after-transition.json",
        ):
            read_object(source / name, config.benchmark.install.max_receipt_bytes)
            shutil.copyfile(source / name, work / name)
        marker = "candidate-after-audit-line"
        read_bytes(source / marker, config.benchmark.install.max_receipt_bytes)
        shutil.copyfile(source / marker, work / marker)
        return work / "report.json"
    owner_path = config.path(config.benchmark.install.macos_owner)
    sys.path.insert(0, str(owner_path.parent))
    spec = importlib.util.spec_from_file_location("capsem_owned_tart", owner_path)
    if spec is None or spec.loader is None:
        raise ValueError("existing Tart owner is unavailable")
    tart_owner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(tart_owner)
    previous = sys.argv
    sys.argv = [str(owner_path), *tart_arguments(prepared, work)]
    try:
        if tart_owner.main() != 0:
            raise RuntimeError("existing Tart owner did not settle successfully")
    finally:
        sys.argv = previous
    return work / "report.json"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", required=True, type=Path)
    parser.add_argument("--source-commit", required=True, type=SourceCommit)
    parser.add_argument("--work-dir", required=True, type=Path)
    args = parser.parse_args()
    config = gate_config.load(ROOT)
    settings = config.benchmark.install
    if not args.work_dir.is_absolute() or not args.work_dir.resolve().is_relative_to(
        config.path(settings.work_root).resolve()
    ):
        raise ValueError("Install owner output must stay in its configured benchmark generation")
    if source_commit_for_checkout(ROOT) != args.source_commit:
        raise ValueError("Install owner source differs from the recorded commit")
    platform = "macos" if sys.platform == "darwin" else sys.platform
    prepared = load_inputs(
        args.inputs,
        maximum=settings.max_input_bytes,
        package_maximum=settings.max_package_bytes,
        source_commit=args.source_commit,
        platform=platform,
        architecture=config.host_arch().dpkg,
    )
    with unwind_sigterm():
        invoke(ROOT, prepared, args.work_dir, Runner(ROOT, stream=sys.stderr))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
