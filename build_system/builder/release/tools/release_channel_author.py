"""Release-owned candidate channel authoring and cohort resolution by digest.

The half of a rehearsal that imitates nothing: `capsem-admin` authors the
channel and `fetch-release-artifacts.py` resolves it, which are the same binary
and the same script a real release runs. Only the URLs are local, because a
local run has nowhere else to put bytes it has not published.

Split from `release_cohort`, which decides what a cohort *is*. This decides how
a channel becomes one, and it is where the input that keeps being got wrong
lives: why its manifest has to be served rather than named as a file.
"""

from __future__ import annotations

import os
import subprocess

from capsem_builder.gate.releaseauthoring import author_native_candidate
from capsem_builder.gate.sourcecommit import source_commit_for_checkout

from . import local_release_glowup, repository_root

PROJECT_ROOT = repository_root()


def glowup_helpers():
    """Return the package-owned staging helpers used by the real release."""
    return local_release_glowup


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
    )

    # From here nothing is rehearsal-specific: this is the composite action the
    # pairing job runs, against the manifest just authored rather than one a
    # channel published.
    run(
        [
            "uv",
            "run",
            "python",
            "build_system/scripts/release/fetch-release-artifacts.py",
            "--manifest-url",
            f"{base_url}/assets/{args.channel}/{config.install.manifest_name}",
            "--kind",
            "runtime",
            "--architecture",
            config.host_arch().name,
            "--output",
            str(inputs),
        ]
    )
