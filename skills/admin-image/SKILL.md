---
name: admin-image
description: Capsem profile-derived image building and verification. Use this whenever the user edits capsem-admin image plan/build-workspace/build/verify/sbom, package or tool inventories, doctor bundles, profile-owned VM assets, rootfs/kernel/initrd build lanes, or asks how corporate admins build and prove images from profiles.
---

# Admin Image

Use this skill for profile-derived VM image tooling. The profile is the source
of truth; image workspaces, build inputs, package contracts, inventories, SBOMs,
and doctor-bundle verification are derived proof artifacts.

## Ground Rules

- Never make `guest/config` or hand-edited image settings the authority for
  admin workflows. It is only a bridge where current builders still need it.
- Default `--arch` is `all`; single-arch narrowing is for local debugging or CI
  shards.
- Verification must fail closed on missing assets, hash drift, inventory drift,
  SBOM failure, or doctor-bundle failures.
- Keep image reports typed with Pydantic models and canonical JSON emission.

## First Files To Read

- `src/capsem/builder/image_plan.py`
- `src/capsem/builder/image_workspace.py`
- `src/capsem/builder/image_verify.py`
- `src/capsem/builder/image_sbom.py`
- `scripts/build-assets.sh`
- `docs/src/content/docs/architecture/custom-images.md`

## Admin CLI Surface

Use these commands when working on image pipelines:

```bash
uv run capsem-admin image plan config/profiles/base/coding.profile.toml
uv run capsem-admin image build-workspace config/profiles/base/coding.profile.toml --out /tmp/capsem-image-workspace
uv run capsem-admin image build config/profiles/base/coding.profile.toml --dry-run --json
uv run capsem-admin image verify config/profiles/base/coding.profile.toml --assets-dir assets
uv run capsem-admin image sbom config/profiles/base/coding.profile.toml --assets-dir assets --out-dir /tmp/capsem-sbom
```

## Testing Checklist

- Prove profile-derived workspace files parse back through existing builder
  models until the bridge is removed.
- Prove all-arch and single-arch paths, including missing selected assets.
- Prove inventory checks catch missing packages, required tools, and version
  mismatches.
- Prove doctor bundles are read safely from tar files and fail verification on
  failing JUnit results.
- Prove SBOM output includes profile id, revision, arch, package-contract hash,
  and purl package references.

Useful focused gates:

```bash
uv run python -m pytest tests/test_image_plan.py tests/test_image_workspace.py tests/test_image_verify.py tests/test_image_sbom.py tests/test_admin_cli.py -q
uv run python -m pytest tests/test_build_assets_script.py tests/test_docker.py -q
```
