"""Fabricating a release cohort from what this machine has already built.

The mechanics behind `rehearse-release-cohort.py`, in a module that can be
imported. Split out because the entry point is argparse and this is the part
worth reading: which scripts a release actually runs, in which order, and which
of their inputs are the ones that keep being got wrong.
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path

from capsem_builder.gate import config as gate_config
from release_channel_author import author_and_fetch, glowup_helpers, run

PROJECT_ROOT = Path(__file__).resolve().parents[1]


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


def unpublished_before(channel: str, directory: Path) -> Path:
    """The public before-state of a channel nobody has released into.

    A first release pairs the candidate against nothing, and nothing still has
    to arrive as a verified cohort: an empty profile set the fetcher was told to
    accept, and a report that reproduces it.

    This shape is not invented. `select-runtime-preflight-manifest.py` reports
    `bootstrap=true` for the live stable channel, so the lane projects its
    before-state with `project-first-channel-before.py`, and that projection was
    run against the live channel and returns exactly `packages: []` and
    `profiles: {}`. That is what makes the pairing `FRESH_INSTALL`, which is the
    pairing a first release makes and the one nothing had ever exercised.
    """

    def write(name: str, document: dict) -> Path:
        path = directory / name
        path.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        return path

    directory.mkdir(parents=True, exist_ok=True)
    manifest = write("manifest.json", {"channel": channel, "profiles": {}, "packages": []})
    write(
        "release-inputs.json",
        {
            "schema": "capsem.release_inputs.v1",
            "kind": "profiles",
            "manifest_url": manifest.as_uri(),
            "output": str(directory),
            "artifacts": [],
            "allow_empty_profiles": True,
        },
    )
    return manifest


def build_cohort(args) -> dict[str, str]:
    """Author a candidate channel, resolve it by digest, and stage it.

    Returns what the plan's later steps were built to name, so a reader of the
    run log can see that the paths a step was given are the paths this wrote.
    """
    config = gate_config.load(PROJECT_ROOT)
    helpers = glowup_helpers()
    admin = args.bin_dir / "capsem-admin"
    if not admin.is_file():
        raise SystemExit(f"the rehearsal authors its manifest with {admin}, which is not built")

    # Absolute throughout: every URL this authors is a `file://` one, and a
    # relative path in a URL is not a location at all.
    work, inputs, workspace = (
        path.resolve() for path in (args.work_dir, args.inputs_dir, args.content_root)
    )
    args.package = args.package.resolve()
    args.before_inputs = args.before_inputs.resolve()
    for path in (work, inputs, workspace, args.package.parent, args.before_inputs):
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
    run(
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
        author_and_fetch(
            args,
            config,
            helpers,
            base_url=base_url,
            dist=dist,
            paths=(exact, sbom, manifests, inputs, admin),
        )
    run(
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
    run(
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
    before = unpublished_before(args.channel, args.before_inputs)
    return {
        "before_manifest": str(before),
        "before_profile_inputs": str(args.before_inputs),
        "schema": "capsem.release_rehearsal.v1",
        "channel": args.channel,
        "version": helpers.deb_version(exact),
        "manifest": str(dist / "assets" / args.channel / config.install.manifest_name),
        "inputs": str(inputs),
        "package": str(args.package),
        "content_root": str(workspace),
    }
