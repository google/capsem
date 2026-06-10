# Plan: Repo Ontology Cleanup

## Goal

Make the repository layout match Capsem's architecture:

- host/profile/corp/settings config under `config/host/`;
- Docker/build templates under `config/docker/`;
- profile-owned payload source under `config/profiles/<profile_id>/`;
- guest filesystem seed under `config/profiles/<profile_id>/root/`;
- generated runtime config under `target/config/`;
- built assets/packages as generated artifacts.

This sprint starts as an inventory and plan because moving these paths without
a contract would create exactly the kind of parallel system we are trying to
burn.

## Key Decisions

- `config/` owns all configuration-shaped source.
- `config/host/` owns profile, corp, settings, enforcement, detection, plugin,
  and UI settings contracts.
- `config/docker/` owns Dockerfile/build templates. These templates are hashed
  build inputs, not Python source.
- The profile is the only ledger. Packages, MCP, root seed, assets, rules,
  plugins, and VM defaults must be declared or referenced by the profile.
- There is no `config/guest`. Profile-owned payloads live under
  `config/profiles/<profile_id>/`.
- Profiles are directories. The profile ledger is
  `config/profiles/<profile_id>/profile.toml`, not
  `config/profiles/<profile_id>.toml`.
- `config/profiles/<profile_id>/root/` is a guest `/` filesystem seed.
- Any profile-owned file that influences runtime behavior must be hash-pinned
  from the profile ledger and shipped in the package manifest.
- Runtime `/root` must be populated by `capsem-init` after mounts because `/root`
  is tmpfs.
- No old-path compatibility once the move lands.

## Initial File/Directory Changes

- Move only surviving payload content from `guest/config/**` to
  `config/profiles/<profile_id>/**`, and only when that profile owns it.
- Delete/rewrite obsolete provider/network/defaults-shaped image config.
- Move current host config files into `config/host/**`.
- Move Docker templates from `src/capsem/builder/templates/**` to
  `config/docker/**`.
- Move `config/profiles/code.toml` to `config/profiles/code/profile.toml`.
- Add `config/profiles/code/mcp.json`.
- Add `config/profiles/code/apt-packages.txt`.
- Add `config/profiles/code/python-requirements.txt`.
- Add `config/profiles/code/npm-packages.txt`.
- Add `config/profiles/code/install.sh`.
- Add `config/profiles/code/tips.txt`.
- Add `config/profiles/code/root/**`.
- Add profile file-reference schema entries with path, blake3, size, and kind.
- Replace builder path defaults from `guest/config` with admin-resolved inputs.
- Replace Python `GuestImageConfig` with an image-backend spec that cannot
  describe host/profile/provider policy.
- Delete Python scaffolding commands that create AI providers, MCP servers, or
  guest config projects.
- Remove `generate_defaults_json()` dependency on guest image config.
- Update `capsem-admin` path defaults and just recipes.
- Update docs and skills that mention `guest/config`.
- Resolve every item in the magic inventory: generated config JSON, MCP tool
  exports, root dot-shims, old env overrides, stale squashfs outputs, and
  dev-install bypass scripts.

## Testing Matrix

- Unit/contract:
  - path resolver tests for `config/host`, profile directories, and
    profile-owned root;
  - profile/corp/settings parse tests from new paths;
- package file parser tests for apt/Python/Node files;
- installer script hash/path validation tests;
  - MCP JSON parser/validator tests.
- Functional:
  - `capsem-admin profile validate`;
  - `capsem-admin profile materialize`;
- `capsem-admin image verify`.
- admin image plan proves every backend input explicitly; no opaque guest dir.
- backend/CI build ledger includes rendered Dockerfile, build context,
  exported rootfs tar, final EROFS, kernel assets, tool-version output,
  compression settings, git revision, and project version.
- admin/profile build ledger includes profile and profile-owned payload hashes
  once profile file references are hash-pinned in `profile.toml`.
- magic inventory gate proves no live production references to old ontology
  remain.
- `capsem-admin profile check` rejects a mutated enforcement/detection/MCP/
  package/root/tips file whose hash no longer matches `profile.toml`.
- Adversarial:
- old path rejected;
- Python builder cannot accept AI-provider/network/MCP/VM-settings fields in its
  image spec;
- checked-in credential-like secrets rejected under
  `config/profiles/<profile_id>/root`;
  - root seed path traversal rejected.
- E2E/VM:
  - `capsem-doctor` confirms Codex/Gemini/Claude config files exist in runtime
    `/root`;
  - local MCP config is usable from inside the VM.
- Performance:
  - no runtime hot-path regression expected;
  - image build refresh path measured only if package refresh behavior changes.

## Done

- Directory ontology is documented.
- Code uses one path for each concept.
- Docker templates live under `config/docker/`.
- `guest/config` is gone as a product concept.
- `config/guest` does not exist.
- The profile can fully explain why every image-baked package, MCP declaration,
  asset, plugin, rule file, and root seed input exists.
- The installed/package manifest can reproduce and verify the profile ledger:
  profile hash plus referenced file hashes.
- AI/tool config files are real guest seed files, not inline TOML theater.
- The VM boots and doctor proves the seed projection.
- Docs and skills no longer teach stale paths.
- Magic inventory is empty or every surviving item is explicitly documented as
  a generated output, core guest payload, or developer shim.
