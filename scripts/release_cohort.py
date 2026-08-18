"""Fabricating a release cohort from what this machine has already built.

The mechanics behind `rehearse-release-cohort.py`, in a module that can be
imported. Split out because the entry point is argparse and this is the part
worth reading: which scripts a release actually runs, in which order, and which
of their inputs are the ones that keep being got wrong.
"""

from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import sys
from pathlib import Path

from capsem.gate import config as gate_config
from capsem.gate.releaseauthoring import author_native_candidate
from capsem.gate.sourcecommit import source_commit_for_checkout

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _glowup_helpers():
    """The staging helpers, from the script that already owns them.

    Loaded by path because the file is named with hyphens and cannot be
    imported. Copying the three functions instead would put a second
    implementation of the asset layout beside the one a release depends on, and
    a rehearsal that stages artifacts differently from the thing it rehearses
    is worse than no rehearsal.
    """
    path = PROJECT_ROOT / "scripts/local-release-glowup.py"
    spec = importlib.util.spec_from_file_location("capsem_local_release_glowup", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot load release staging helpers from {path}")
    module = importlib.util.module_from_spec(spec)
    # Registered before it is executed. `@dataclass` resolves `cls.__module__`
    # through `sys.modules`, so a module that is not there yet raises inside
    # the decorator rather than anywhere near the import.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _source_profiles(config) -> Path:
    """The profile directory a release graph is authored from.

    The checkout's, not the materialized copy. A graph records profile config by
    source path and the site serves those exact bytes from the source ref, so
    authoring from materialized output produces a channel whose config nothing
    can reproduce -- and whose staged profiles `materialize-config` then
    refuses, because they already carry the pins it exists to add. Read off
    `profiles_glob` rather than spelled again: one value, one answer.
    """
    return PROJECT_ROOT / Path(config.assets.profiles_glob).parent.parent


def _run(command: list[str], *, env: dict[str, str] | None = None) -> None:
    subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        check=True,
        env=None if env is None else {**os.environ, **env},
    )


def _candidate_package(config, packages_dir: Path) -> Path:
    """The release-mode package this run built for the host architecture."""
    suffix = f"_{config.host_arch().dpkg}.deb"
    built = sorted(path for path in packages_dir.rglob("*.deb") if path.name.endswith(suffix))
    if not built:
        raise SystemExit(
            f"no {suffix} package under {packages_dir}; the rehearsal proves the "
            "package the local lane built, so it has to run after it"
        )
    return built[-1]


def _author_and_fetch(args, config, helpers, *, base_url, dist, paths) -> None:
    """Author the candidate channel and resolve its cohort, exactly as CI does.

    One function because both halves need the server alive: the manifest
    records the URLs, and the fetch is what proves they resolve.
    """
    exact, sbom, manifests, inputs, admin = paths
    version = helpers.deb_version(exact)
    release_dir = dist / "releases" / "download" / args.channel / f"v{version}"
    for artifact in (exact, sbom):
        helpers.copy_artifact_tree(artifact, release_dir / artifact.name)

    source_manifest = manifests / f"{args.channel}-assets-manifest.json"
    helpers.clone_manifest_for_channel(
        args.assets_dir / "manifest.json", source_manifest, args.channel
    )
    helpers.stage_manifest_artifacts(source_manifest, args.assets_dir, dist, base_url)

    graph = dist / "assets" / args.channel / config.install.manifest_name
    author_native_candidate(
        source_manifest,
        runner=lambda command, env=None: _run(command, env=env),
        admin=admin,
        assets_dir=args.assets_dir,
        profiles_dir=_source_profiles(config),
        channel=args.channel,
        version=version,
        source_commit=source_commit_for_checkout(PROJECT_ROOT),
        artifacts=(exact, sbom),
        release_environment=config.environment.release_site.runtime(
            url=f"{base_url}/releases/download/{args.channel}"
        ),
        asset_source_base=f"{base_url}/assets/releases/{{asset_version}}",
        dist=dist,
        graph_manifest=graph,
        manifest_version=config.install.manifest_version,
        profile_revision_policy=config.install.profile_revision_policy,
    )
    # A graph records profile config as site-absolute `/profiles/releases/...`
    # paths and does not carry the bytes. The deployed site materializes them
    # from the source ref; here the source is this checkout.
    _run(
        [
            "uv",
            "run",
            "python",
            "scripts/materialize-graph-profile-artifacts.py",
            "--dist",
            str(dist),
            "--channel",
            args.channel,
            "--source-root",
            str(PROJECT_ROOT),
        ]
    )

    # From here nothing is rehearsal-specific: this is the composite action the
    # pairing job runs, against the manifest just authored rather than one a
    # channel published.
    _run(
        [
            "uv",
            "run",
            "python",
            "scripts/fetch-release-artifacts.py",
            "--manifest-url",
            f"{base_url}/assets/{args.channel}/{config.install.manifest_name}",
            "--kind",
            "profiles",
            "--architecture",
            config.host_arch().name,
            "--output",
            str(inputs),
        ]
    )


def build_cohort(args) -> dict[str, str]:
    """Author a candidate channel, resolve it by digest, and stage it.

    Returns what the plan's later steps were built to name, so a reader of the
    run log can see that the paths a step was given are the paths this wrote.
    """
    config = gate_config.load(PROJECT_ROOT)
    helpers = _glowup_helpers()
    admin = args.bin_dir / "capsem-admin"
    if not admin.is_file():
        raise SystemExit(f"the rehearsal authors its manifest with {admin}, which is not built")

    # Absolute throughout: every URL this authors is a `file://` one, and a
    # relative path in a URL is not a location at all.
    work, inputs, workspace = (
        path.resolve() for path in (args.work_dir, args.inputs_dir, args.content_root)
    )
    args.package = args.package.resolve()
    for path in (work, inputs, workspace, args.package.parent):
        if path.exists():
            shutil.rmtree(path)
    dist, manifests = work / "dist", work / "manifests"
    for path in (dist, manifests, workspace, args.package.parent):
        path.mkdir(parents=True)

    # Authored under the name the rail gave it, and only then copied to the
    # fixed path the plan was built against. Both are needed and neither will
    # do alone: `capsem-admin assets channel record-binary` refuses a package
    # whose filename does not carry the version, and a step argument that
    # changes with the version is one the dry run cannot print.
    built = _candidate_package(config, args.packages_dir)
    version = helpers.deb_version(built)
    exact = work / "artifacts" / f"v{version}" / built.name
    exact.parent.mkdir(parents=True)
    shutil.copy2(built, exact)
    sbom = exact.parent / "capsem-sbom.spdx.json"
    _run(
        [
            "uv",
            "run",
            "python",
            "scripts/generate-host-binary-sbom.py",
            "--output",
            str(sbom),
            str(exact),
        ]
    )

    # Over loopback rather than `file://`: a graph records profile config as
    # site-absolute `/profiles/releases/...` paths, and `urljoin` resolves those
    # against the manifest's own URL -- under `file://` that is the root of the
    # filesystem. An HTTP root is the only way to say "the site root is here".
    with helpers.local_release_server(dist) as base_url:
        _author_and_fetch(
            args,
            config,
            helpers,
            base_url=base_url,
            dist=dist,
            paths=(exact, sbom, manifests, inputs, admin),
        )
    _run(
        [
            "uv",
            "run",
            "python",
            "scripts/stage-release-test-inputs.py",
            "--input-dir",
            str(inputs),
            "--assets-dir",
            str(workspace / config.functional.assets_dir),
            "--config-root",
            str(work / "release-config"),
            "--shared-config-root",
            "config",
        ]
    )
    _run(
        # `--pair-content`, as every lane that runs `glowup.content` must. That
        # step compares the staged asset manifest against the materialized
        # runtime one byte for byte, and only this flag makes them the same
        # document -- the staged one is the channel graph until it is paired.
        ["bash", "scripts/materialize-config.sh", "--pair-content"],
        env={
            # A path rather than a `file://` URL, and the assets directory
            # beside it. `--pair-content` compares the two as filesystem paths
            # to check it is pairing the manifest it was selected with, so a
            # URL here fails that comparison against itself.
            "CAPSEM_ASSET_MANIFEST": str(
                workspace / config.functional.assets_dir / config.install.manifest_name
            ),
            "CAPSEM_ASSETS_PATH": str(workspace / config.functional.assets_dir),
            "CAPSEM_CONFIG_ROOT": str(work / "release-config"),
            "CAPSEM_CONFIG_OUTPUT_ROOT": str(workspace / config.functional.config_root),
        },
    )

    shutil.copy2(exact, args.package)
    return {
        "schema": "capsem.release_rehearsal.v1",
        "channel": args.channel,
        "version": helpers.deb_version(exact),
        "manifest": str(dist / "assets" / args.channel / config.install.manifest_name),
        "inputs": str(inputs),
        "package": str(args.package),
        "content_root": str(workspace),
    }
