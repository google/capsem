"""Release-owned candidate channel authoring and cohort resolution by digest.

The half of a rehearsal that imitates nothing: `capsem-admin` authors the
channel and `fetch-release-artifacts.py` resolves it, which are the same binary
and the same script a real release runs. Only the URLs are local, because a
local run has nowhere else to put bytes it has not published.

Split from `release_cohort`, which decides what a cohort *is*. This decides how
a channel becomes one, and it is where the inputs that keep being got wrong
live: which profile directory a graph is authored from, and why its manifest has
to be served rather than named as a file.
"""

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
from pathlib import Path

from capsem_builder.gate.releaseauthoring import author_native_candidate
from capsem_builder.gate.sourcecommit import source_commit_for_checkout

from . import repository_root

PROJECT_ROOT = repository_root()


def glowup_helpers():
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


def run(command: list[str], *, env: dict[str, str] | None = None) -> None:
    subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        check=True,
        env=None if env is None else {**os.environ, **env},
    )


def author_and_fetch(args, config, helpers, *, base_url, dist, paths) -> None:
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
        runner=lambda command, env=None: run(command, env=env),
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
    run(
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
    run(
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
