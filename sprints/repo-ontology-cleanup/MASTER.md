# Repo Ontology Cleanup

Status: Planning

## Why This Exists

Capsem has grown from a single VM prototype into a profile-owned, multi-VM,
security-led runtime. The repository layout did not keep up. Configuration,
guest image inputs, generated build outputs, local install artifacts, and
developer tool shims now live close enough together that it is easy to patch the
wrong layer and accidentally create a second truth.

The immediate bug is AI/tool configuration: Codex, Claude, Gemini, and future
tools need files inside the guest runtime, but the current tree has inline
`guest/config/ai/*.toml` file declarations that are not actually projected into
runtime `/root`. The broader problem is ontology: we need every directory to
say clearly whether it is source, generated output, package artifact, runtime
state, or documentation.

## Target Ontology

| Domain | Target Path | Meaning |
| --- | --- | --- |
| Host config source | `config/host/` | Checked-in source for host-side contracts: profiles, corp, settings, enforcement rules, Sigma detections, plugin config, UI settings contract. |
| Docker/build templates | `config/docker/` | Checked-in Dockerfile and build templates used to produce VM assets. Templates are configuration inputs and must be hashed in build records. |
| Profile source | `config/profiles/<profile_id>/profile.toml` plus sibling files | Checked-in profile ledger and all profile-owned payloads. If a package, MCP server, rule file, detection file, asset, VM default, or packaged root file is not in the profile, it does not exist. |
| Profile packaged root | `config/profiles/<profile_id>/root/` | Profile-owned filesystem tree representing guest `/`. Example: `config/profiles/code/root/root/.codex/config.toml` maps to runtime `/root/.codex/config.toml`. |
| Guest embedded artifacts | `guest/artifacts/` or successor | Checked-in executable/script payloads that are copied into initrd/rootfs, such as `capsem-init`, doctor, benchmarks, diagnostics. This may move later, but it is distinct from config. |
| Generated runtime config | `target/config/` | Materialized local build config created by `capsem-admin`, never hand-edited, never source truth. |
| Built VM assets | `assets/` | Generated kernel/initrd/rootfs/manifest output. Large, ignored, package input only. |
| Built packages | `packages/` | Generated `.pkg`/`.deb` installers. Large, ignored, release/dev install output only. |
| Runtime install state | `~/.capsem/` | User machine state, not repository source. |
| Tests | `tests/` | Host-side tests. Guest diagnostics stay in the guest payload area. |
| Benchmarks | `benchmarks/` plus guest bench payload | Host benchmark harnesses and source definitions. Generated benchmark results should live under `target/` or explicit ignored artifact directories. |

## Root Seed Contract

`config/profiles/<profile_id>/root/` is a profile-owned guest filesystem seed,
not a global guest config directory. It only participates in an image when the
active profile hard-references it as packaged root input.

Examples:

| Desired guest path | Checked-in source path |
| --- | --- |
| `/root/.codex/config.toml` | `config/profiles/code/root/root/.codex/config.toml` |
| `/root/.claude/settings.json` | `config/profiles/code/root/root/.claude/settings.json` |
| `/root/.gemini/settings.json` | `config/profiles/code/root/root/.gemini/settings.json` |
| `/etc/capsem/something.conf` | `config/profiles/code/root/etc/capsem/something.conf` |

Target code profile layout:

```text
config/profiles/
  code/
    profile.toml
    enforcement.toml
    detection.yaml
    mcp.json
    apt-packages.txt
    python-requirements.txt
    npm-packages.txt
    install.sh
    tips.txt
    root/
      root/.codex/config.toml
      root/.claude/settings.json
      root/.gemini/settings.json
```

`profile.toml` is the ledger. The sibling files are payload only. They are valid
only because `profile.toml` references them and binds their blake3 hashes.

Build rule:

- The profile is the only ledger. Packages, MCP, assets, rules, plugins, VM
  defaults, and root seed inputs must be declared or referenced by the profile.
- If it is not in the profile, it does not exist.
- Every profile-owned sibling file that affects runtime behavior must be
  hash-pinned from `profile.toml` or bound by the generated manifest before it
  is accepted by admin/service/runtime.
- The package manifest must ship the profile ledger and its referenced files
  together with their hashes, so installed systems can report and verify the
  exact profile payload they run.
- The build ledger must also record what actually lands in the VM: declared
  package input hashes, installed package names, installed versions, and local
  package/artifact hashes when apt, Python/uv, npm, or a manual installer gives
  us enough local metadata to compute them. The release/debug answer must be
  "this is what is running in the VM", not "this is what the profile requested."
- Preferred OBOM generator: `cdxgen/cdxgen` using its CycloneDX OBOM path
  (`obom`, equivalent to `cdxgen -t os`) against the produced Linux rootfs or
  image. Capsem can enrich that document with profile id, profile hash, asset
  hash, build-ledger hash, and cdxgen version, but it must not invent a
  parallel package inventory format unless cdxgen is unavailable in a local dev
  smoke path.
- The builder copies `config/profiles/<profile_id>/root/` into a stable seed
  path inside the rootfs, not directly into runtime `/root`.
- `capsem-init` copies the seed into runtime `/` after tmpfs/overlay mounts are
  ready.
- This is mandatory because runtime `/root` is tmpfs; files baked directly into
  rootfs `/root` can be hidden.
- No credentials are checked into this tree. Credential values still belong to
  the credential broker/keychain path.

## Current Inventory Summary

| Current Path | Used? | Current Meaning | Problem | Target |
| --- | --- | --- | --- | --- |
| `config/` | Yes | Mixed host source config plus generated/default artifacts | Host profile/corp/settings are mixed with generated schema/defaults/pricing and test fixtures. | Split into `config/host/`, `config/generated-source/` only if truly checked in, and `config/test-fixtures/` if needed. |
| `guest/config/` | Yes | Guest image config consumed by Python builder | This violates the profile-ledger contract. It makes packages/MCP/provider/network/image inputs exist outside the profile. | Delete as authority. Move surviving data into profile declarations or profile-owned payload files under `config/profiles/<profile_id>/`. |
| `guest/config/ai/*.toml` | Partially | AI CLI metadata and inline config file declarations | Invalid ontology. There are no AI providers as image/config authorities. Tool packages must be profile package declarations; config files must be profile-referenced root seed files. | Delete. |
| `guest/config/mcp/local.toml` | Partially | Built-in MCP metadata | Invalid unless represented in the profile. MCP lives in profile or it does not exist. | Move MCP declarations to profile; CLI bootstrap can be root seed content only when profile references it. |
| `guest/artifacts/` | Yes | Init, doctor, diagnostics, guest benchmarks, tips | Contains executable guest payload, not config. Name is acceptable but should be documented as payload. | Keep or later move to `guest/payload/`; not part of this sprint unless needed. |
| `guest/artifacts/tips.txt` | Yes | Guest login tips. | It is profile experience content, not global guest artifact. | Move to `config/profiles/code/tips.txt` and hash-pin from profile. |
| `src/capsem/builder/templates/` | Yes | Dockerfile templates used to build kernel/rootfs. | Hidden build config inside Python source; admin/profile cannot hash or explain it as a build input. | Move to `config/docker/` and include template hashes in build plan/build record. |
| `src/capsem/builder/` | Yes | Python builder package | Reads `guest/config/` and renders rootfs/kernel Dockerfiles. | Demote to implementation backend. It should receive a profile-derived image spec and cannot discover packages, MCP, packaged root, or settings on its own. |
| `crates/capsem-admin/` | Yes | Rust admin orchestration CLI | Orchestrates image/profile/manifest; must be the single route for materialization. | Promote to owner of profile-led image build contract. It resolves the profile ledger and invokes the backend with explicit inputs. |
| `target/config/` | Yes generated | Materialized runtime config | Correct idea, but easy to confuse with checked-in `config/`. | Keep as generated output; docs/tests must reinforce. |
| `assets/` | Yes generated | VM assets and manifest | Large generated output; correctly ignored, but visible at repo root. | Keep or later move to `target/assets`; for 1.3 avoid moving package assumptions unless necessary. |
| `packages/` | Yes generated | Built installers | Correctly ignored. | Keep generated. |
| `.claude/`, `.codex/`, `.gemini/` | Yes tracked shims | Local agent-tool compatibility shims/settings | Dot dirs at repo root look like runtime config. | Keep only as symlinks/settings required by tools, document as developer shims, never product config. |
| `frontend/`, `docs/`, `site/` | Yes | UI, docs site, marketing site | Generated `node_modules`, `.astro`, `dist` make inventory noisy. | Source stays; generated dirs ignored and excluded from ontology docs. |
| `sprints/` | Yes | Planning/history | Large but useful. | Keep. New sprint docs must be self-contained. |

## Remaining Magic Inventory

These are known non-ledger or hidden-input paths found by search. They are not
all equally bad, but each needs an explicit keep/move/delete decision.

| Magic | Evidence | Why It Is Suspicious | Target Decision |
| --- | --- | --- | --- |
| `guest/config/**` | Builder, tests, docs, skills, justfile. | Shadow profile/image config authority. | Delete as authority; profile ledger replaces it. |
| `src/capsem/builder/templates/*.j2` | Rendered by Python builder. | Hidden Docker build input in source package. | Move to `config/docker/`; hash in build record. |
| `config/defaults.json` | Embedded by Rust registry; generated from guest TOML. | Generated checked-in settings truth derived from wrong source. | Replace generation from `config/host/settings.toml`; decide whether checked-in generated JSON remains needed. |
| `config/settings-schema.json` | Generated schema. | Checked-in generated artifact may drift. | Keep only if release process needs checked-in schema; otherwise generate under `target/config`. |
| `config/mcp-tools.json` | Generated by `mcp_export`. | Global MCP tool data outside profile ledger. | Move to profile-owned MCP/tool manifest or generated `target/config`; no global MCP truth. |
| `guest/artifacts/tips.txt` | Copied into rootfs. | Profile experience content outside profile. | Move to `config/profiles/code/tips.txt`. |
| `guest/artifacts/capsem-bashrc` | Copied into `/etc/capsem-bashrc`; agent uses it. | Shell behavior outside profile root/ledger. | Decide: profile root file or core guest payload. If profile-specific, move/hash-pin. |
| `guest/artifacts/diagnostics` and `capsem-doctor` | Baked into rootfs. | Guest test payload, likely core not profile. | Keep as guest payload, but build record must hash it. |
| Root `.gemini/settings.json` | Tracked root dotfile. | Looks like product runtime config at repo root. | Keep only as developer shim if required; document or move under dev tooling. |
| Root `.claude/`, `.codex/`, `.gemini/` | Tracked symlinks/shims/settings. | Developer-tool shims at root look like runtime config. | Keep only if required; document as dev shims, not product config. |
| `CAPSEM_USER_CONFIG` / `CAPSEM_CORP_CONFIG` | Loader env overrides and tests. | Old settings/corp path model; can bypass profile/corp ontology if used in production. | Restrict to tests/dev or replace with profile/corp roots consistent with new contract. |
| `CAPSEM_PROFILES_DIR` | Service/dev justfile. | Useful generated runtime profile selector, but must point at `target/config`/installed profile dirs only. | Keep, but rename/restrict if needed. |
| `assets/current` | Justfile and builder symlink/copy. | Generated convenience alias, can hide real arch/hash. | Keep only as package/frontend build compatibility if verified; never ledger truth. |
| `rootfs.squashfs` artifacts | Assets and tests mention stale fallback. | EROFS is contract; stale files confuse boot/debug. | Delete generated stale files; keep only negative tests that reject squashfs-only manifests. |
| `scripts/simulate-install.sh` / `sync-dev-assets.sh` | Install tests still reference. | Dev/install bypass rails can drift from package/admin install path. | Either delete or clearly demote to tests using same admin/package logic. |
| `manifest-origin.json` | Package provenance. | Useful, but asset-only; profile files need analogous provenance. | Keep and extend package manifest/provenance to profile payloads. |

Rule for this sprint: a path is allowed only if it is one of:

- profile ledger/payload under `config/profiles/<id>/`;
- host/corp/settings source under `config/host/`;
- Docker/build template under `config/docker/`;
- core guest payload with build-record hash;
- generated output under `target/`, `assets/`, or `packages/`;
- explicitly documented developer shim.

## Work Slices

### S0: Freeze Current State

- [x] Preserve current dirty install-log/version-stamp work in a commit or an
  explicit parked patch before moving paths.
- [x] Record this ontology in sprint docs before code moves.
- [x] Add a guardrail note to active finalizing sprint: ontology cleanup must
  complete before guest AI config/root seed work.

### S1: Profile-Ledger Image Input Contract

- [ ] Move host config source into `config/host/`.
- [x] Move Dockerfile/build templates from `src/capsem/builder/templates/` to
  `config/docker/`.
- [x] Move `config/profiles/code.toml` to `config/profiles/code/profile.toml`.
- [x] Extend/confirm profile schema owns all image-baked packages.
- [x] Extend/confirm profile schema owns all MCP declarations.
- [x] Extend/confirm profile schema owns packaged root path under
  `config/profiles/<profile_id>/root/`.
- [x] Extend profile schema with hash-pinned file references for enforcement,
  detection, MCP, `apt-packages.txt`, `python-requirements.txt`,
  `npm-packages.txt`, `install.sh`, packaged root, and tips.
- [ ] Replace `capsem-admin --guest-dir guest` with explicit admin-resolved
  profile-derived image inputs.
- [x] Add backend/CI build ledger emission for rendered Dockerfile, build
  context, rootfs tar, final EROFS, kernel assets, tool versions, compression
  settings, git revision, and project version.
- [x] Restore Linux KVM guest-memory safety hardening from the lost Linux work:
  `0422a6ec` full guest physical range validation and `45800223` checked guest
  memory offset arithmetic are ported into current KVM memory/virtio-blk code.
- [ ] Validate AGY/Antigravity by booting the rebuilt profile and running the
  tool inside the guest. Do not raise VM RAM caps speculatively; capture the
  exact kernel/runtime failure and fix the specific guest kernel option if AGY
  still fails.
- [ ] Extend the ledger to hash profile and profile-owned payload files after
  the profile file-reference schema lands.
- [ ] Demote `capsem-builder` to a backend that consumes the admin image spec.
- [ ] Remove product-authoring commands from the Python builder:
  `init`, `new`, `add ai-provider`, `add mcp`, and template scaffolding.
- [ ] Move surviving guest payload files from `guest/config/` into
  profile-owned `config/profiles/<profile_id>/` paths.
- [ ] Delete or reject obsolete `guest/config` provider/network/defaults shape.
- [ ] Split Python models into backend-only image models:
  build architecture, resolved package install sets, resolved tool install sets,
  kernel defconfigs, and resolved root seed metadata. Remove AI provider, MCP
  server, web security, VM settings, and defaults-generator ownership from the
  builder.
- [ ] Move settings/default generation away from `GuestImageConfig`; host
  settings come from `config/host/settings.toml`, profile/corp/rules from
  `config/host`, not guest image TOML.
- [ ] Resolve `config/defaults.json`, `settings-schema.json`, and
  `mcp-tools.json`: move generation source to host/profile truth, or move
  generated outputs under `target/config`.
- [x] Classify root dot-directories (`.gemini`, `.claude`, `.codex`) as
  developer shims or remove/move them.
- [ ] Classify `CAPSEM_USER_CONFIG` and `CAPSEM_CORP_CONFIG` as test/dev-only
  or replace them with contract-consistent profile/corp roots.
- [ ] Keep `target/config/` as generated runtime config.
- [ ] Remove path fallbacks to old locations once tests are green.

### Python Builder Burn List

| Component | Current Role | Verdict |
| --- | --- | --- |
| `src/capsem/builder/cli.py build` | Builds kernel/rootfs from a guest dir. | Keep as backend entrypoint temporarily, but change input to explicit admin image spec. |
| `src/capsem/builder/templates/*.j2` | Dockerfile templates. | Move to `config/docker/`; Python renders templates but does not own them. |
| `src/capsem/builder/cli.py doctor/validate/inspect` | Inspects guest project config. | Rewrite around admin image spec or demote to internal diagnostics. |
| `src/capsem/builder/cli.py init/new/add` | Scaffolds guest config/projects. | Delete. Product config is authored through `config/host` and `capsem-admin`, not Python. |
| `src/capsem/builder/scaffold.py` | Creates guest configs, AI providers, MCP servers. | Delete unless a tiny internal fixture helper remains under tests. |
| `AiProviderConfig` | Provider/network/key/files model. | Delete. There are no AI providers in this ontology. |
| `McpServerConfig` | MCP server config model. | Delete from image builder. MCP belongs to the profile or it does not exist. |
| `WebSecurityConfig` | HTTP domains/upstream ports. | Delete from image builder unless a low-level redirect-port list is still needed by `capsem-init`; that belongs in an explicit image/network spec. |
| `VmResourcesConfig` | CPU/RAM/session retention/logging. | Delete from image builder. Profiles/VM runtime own this. |
| `VmEnvironmentConfig` | Shell config and TLS paths. | Split: shell files move to `config/guest/root`; TLS/image constants stay backend-owned if needed. |
| `generate_defaults_json` | Derives host UI settings from guest TOML. | Delete/replace. Host settings must come from `config/host/settings.toml`. |
| `mcp_server.py` | MCP wrapper around builder config tools. | Delete unless there is a real admin-backed use case. |

### S2: Guest Root Seed Contract

- [x] Add `config/profiles/code/root/`.
- [x] Move Codex, Claude, Gemini config file contents out of inline TOML and
  into real files under `config/profiles/code/root/root/...`.
- [x] Add Antigravity/AGY profile config seed; current install source still
  requires real image build verification.
- [x] Add `config/profiles/code/tips.txt` and remove profile tips from global
  guest artifacts.
- [x] Builder copies the seed into rootfs under a non-runtime seed path.
- [x] `capsem-init` projects the seed into runtime `/` after tmpfs/overlay setup.
- [ ] Doctor verifies the expected files exist in the VM.

### S3: Tool Install And Refresh Discipline

- [x] Replace legacy AI-provider config with profile-owned package files:
  `apt-packages.txt`, `python-requirements.txt`, and `npm-packages.txt`.
- [x] Add profile-owned `install.sh` for manual shell installers such as Claude
  or AGY when a tool is not representable as apt/Python/npm package input.
- [x] Profile build spec maps those package files into apt, Python/uv, and
  Node/npm install steps, then runs `install.sh` as a hash-pinned profile input.
- [ ] Build ledger records the actually installed apt/Python/npm/manual package
  set with names, versions, declared input hashes, and local package/artifact
  hashes where available.
- [ ] Generate a CycloneDX OBOM with `cdxgen/cdxgen` (`obom` / `cdxgen -t os`)
  for each produced profile/arch rootfs and include its path, hash, generator,
  and generator version in the profile build ledger.
- [x] Profile schema/API/admin materialization know how to carry the generated
  OBOM: it is base-image scope only, has its own BLAKE3 hash, and records the
  rootfs hash it describes.
- [ ] Add an explicit release refresh/cache-bust path for npm/curl/apt tool
  installation.
- [ ] Verify Codex, Claude, Gemini, and AGY versions in doctor output.
- [ ] Ensure local MCP config is present for CLIs that need it.

### S4: Documentation And Skill Cleanup

- [x] Move the canonical skill library to `config/skills/`; remove root
  agent skill symlink shims. Profile/agent injection must copy or mount from
  `config/skills/` explicitly.
- [x] Add `capsem-builder validate-skills config/skills` as a Pydantic-backed
  contract gate for skill directories and `SKILL.md` frontmatter; wire it into
  `just test`, `just smoke`, and CI.
- [ ] Update `config/skills/build-images`, `config/skills/asset-pipeline`,
  `config/skills/dev-capsem`, and relevant testing skills.
- [ ] Update docs architecture pages for config/source/generated/runtime
  separation.
- [ ] Remove stale references to `guest/config/`.
- [ ] Document `config/profiles/<profile_id>/root/` with examples and the
  no-secrets invariant.
- [ ] Update release/install docs and skills to say the final local gate is a
  real admin-driven asset build plus package install, not a dev-only sync path.
- [ ] Document AGY/Antigravity package/config handling through profile-owned
  package/root seed files once the install source is verified.

### S5: Verification Gate

- [ ] Unit/contract tests for path resolution.
- [ ] `capsem-admin profile check` verifies every profile file reference exists,
  matches its blake3 hash, and has a valid schema/content parser.
- [ ] Build record verifies Docker template hashes and rendered Dockerfile hash.
- [ ] `capsem-doctor` reports profile id, profile revision, profile hash, and
  referenced file hashes so support can debug profile payload issues.
- [ ] Builder tests proving root seed files enter the rootfs seed path.
- [ ] Init tests proving seed projection happens after runtime mounts.
- [ ] `capsem-admin image verify` against the new layout.
- [ ] `capsem-doctor` VM proof for AI CLI config and local MCP config.
- [ ] Full profile asset rebuild through the admin/just rail, including
  EROFS/LZ4HC rootfs and build-ledger output.
- [ ] Real package build and install smoke with manifest override support; the
  installed service/UI must report profile readiness from installed state.
- [ ] Linux KVM handoff: run the restored guest-memory range/overflow tests on
  Linux CI/hardware. macOS cannot execute `hypervisor::kvm`; local cross-check
  is blocked without Linux GNU/musl C toolchains.
- [ ] Magic inventory gate: `rg` for `guest/config`,
  `src/capsem/builder/templates`, `config/guest`, `config/profiles/code.toml`,
  and old AI provider config paths returns no live production references.

## Non-Negotiable Invariants

- No second config root and no `config/guest`.
- No unsigned/unhashed profile sibling files.
- No `config/profiles/<id>.toml`; profiles are directories with
  `profile.toml`.
- No compatibility fallback to old paths after the move.
- No checked-in credentials.
- No direct rootfs `/root` assumption; runtime `/root` is tmpfs.
- `capsem-admin` remains the single build/materialization rail.
- Docker templates are checked-in config under `config/docker/`, not hidden
  Python package source.
- UI/settings read host profile/settings contracts; they do not infer product
  text from random generated output.
- Builder receives a profile-derived image spec from admin.
- Generated output stays generated.
- Every surviving magic inventory item has a documented owner and test.
