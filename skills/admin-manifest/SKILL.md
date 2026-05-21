---
name: admin-manifest
description: Capsem signed profile catalog and manifest operations. Use this whenever the user edits capsem-admin manifest generate/check/sign/verify-signature, profile status semantics, profile revisions, minisign verification, fast HEAD checks, full downloads, asset signatures, or corporate catalog publishing.
---

# Admin Manifest

Use this skill for corporate profile catalogs. The manifest lists profile
payload revisions and lifecycle status; each profile owns its VM assets and
signature locations.

## Ground Rules

- Manifest entries identify profiles by stable id plus revision. Do not collapse
  identity and version.
- Status semantics are strict: `active` can install/launch, `deprecated` can
  keep running but should migrate, and `revoked` cannot install or launch.
- Fast checks use metadata and HTTP HEAD. Download checks fetch bytes and prove
  hashes/signatures.
- Use minisign for manifest, profile, and asset signatures. Missing signature
  tooling must fail closed in verification paths.

## First Files To Read

- `src/capsem/builder/manifest.py`
- `src/capsem/admin/cli.py`
- `schemas/capsem.profile-manifest.v2.schema.json`
- `docs/src/content/docs/architecture/profiles.md`
- `docs/src/content/docs/usage/admin-cli.md`

## Admin CLI Surface

Use these commands when working on catalogs:

```bash
uv run capsem-admin manifest generate --profiles config/profiles/base --out /tmp/manifest.json
uv run capsem-admin manifest check /tmp/manifest.json --fast
uv run capsem-admin manifest check /tmp/manifest.json --download --pubkey minisign.pub
uv run capsem-admin manifest sign /tmp/manifest.json --key minisign.key --json
uv run capsem-admin manifest verify-signature /tmp/manifest.json --signature /tmp/manifest.json.minisig --pubkey minisign.pub
```

## Testing Checklist

- Prove duplicate profile id/revision pairs fail.
- Prove lifecycle overrides and current revision selection are deterministic.
- Prove remote fast checks use HEAD and full checks use downloaded bytes.
- Prove bad hashes, missing signatures, and bad minisign signatures fail closed.
- Prove manifest docs use `active`, `deprecated`, and `revoked` consistently.

Useful focused gates:

```bash
uv run python -m pytest tests/test_manifest_generate.py tests/test_manifest_check.py tests/test_manifest_crypto.py -q
```
