# Installation, Verification, and Retry

Read this reference before changing evidence or integrity rules, native package
acceptance, installed manifest/status behavior, public artifact retention,
live-channel validation, retry policy, diagnostic continuation, or Cloudflare
deployment checks.

## Evidence and integrity

The manifest defines channel membership, profiles, compatibility bounds,
packages, binaries, and integrity digests. SBOM, OBOM, existing attestations,
and GitHub workflow logs are the release evidence. Do not add another
provenance or approval document.

Profiles belong to channels. A profile may appear in several channels, one
channel, or no public channel; each channel/profile publication is independent.
Every immutable config, image, evidence, and revision path must include enough
channel/profile identity to prevent stable and nightly from aliasing bytes.

Public graph rules:

- release graphs and local asset manifests are generated through
  `capsem-admin manifest generate`;
- packages are delivery containers;
- per-binary inventory stays under its owning package;
- profiles own config, images, inventory, OBOM, evidence, and their minimum
  compatible Capsem version;
- mutable channel pointers use
  `Cache-Control: no-cache, must-revalidate`;
- immutable artifacts use
  `Cache-Control: public, max-age=31536000, immutable`;
- every fetched artifact is verified by recorded digest before use.

Read `release-graph.md` before changing graph generation or channel
deployment.

## Native installation and platform gates

Native installation is a functional outcome, not a file-existence check:

- macOS CI builds the publishable `.pkg`, signs, notarizes, staples,
  Gatekeeper-checks, installs that exact package, verifies the full binary
  cohort and service, and preserves the local Apple VZ proof boundary;
- Linux CI builds every required `.deb`, installs each host-native exact
  package, verifies package metadata, binaries, service, and command behavior,
  and runs the mandatory guest shell where KVM is available;
- publication depends on both platform rails;
- skipped, optional, source-layout-only, or inspect-only checks do not count;
- `build_system/scripts/release/verify-installed-release.py` verifies the exact installed manifest,
  metadata sidecar, profile readiness, package version, update state, and
  post-mortem retrieval of a preserved failed-session log through the installed
  CLI. The same verifier runs in Linux, native macOS, and Tart package lanes;
- the stateful glow-up proves binary-only, profile-only,
  profile-then-binary, channel switching, tamper rejection, and preservation
  of the previous working state, with Winterfell and full doctor after
  transitions.

GitHub-hosted macOS cannot repeat nested Apple Virtualization.framework guest
boot. Local Apple Silicon `just test` owns that VZ proof. Hosted macOS owns
signing, notarization, stapling, installation, and structural verification of
the final publishable package. Neither substitutes for the other.

Linux install qualification separates four observable graph boundaries:

1. enforce the config-owned Docker cache maximum through the common cache API;
2. materialize locked uv/pnpm and snapshot-owned OS inputs from the exact
   host-platform builder child;
3. build the source image with BuildKit networking disabled;
4. bind the input-keyed local tag to its exact platform-child ID, then smoke
   and run that verified tag with container networking disabled. Containerd
   stores do not necessarily accept the child ID itself as a runnable image.

Only step 2 may use the ordinary BuildKit network. A warm helper must carry the
matching input-key label; source build, smoke, Debian proof, and full systemd
install never repair missing inputs with apt, pnpm, uv sync --project build_system, another build, or
an unverified tag. The shared host-builder is an explicit prerequisite; sealing
that upstream materializer is separate tracked work, so do not describe a cold
daemon as globally one-egress until that prerequisite is also closed.

Selected profile bytes travel beside assets/config in one verified read-only
`ProfileContent` root. Qualification rechecks those immutable inputs inside the
container, extracts `capsem-admin` from the exact package, authors one checked
local graph containing both artifact families, securely hands it to postinst,
and executes one `dpkg -i`. The standalone Debian proof uses that same graph
primitive. Native release jobs prepare the package's exact dependency
constraints from the shared immutable Ubuntu snapshot; `apt-get install -f`
and ambient runner indexes are not release proof.

The installed source of truth remains the exact verified
`~/.capsem/assets/manifest.json`, byte-for-byte. Installation and update code must not
rewrite it into a reduced runtime schema. The only metadata sidecar is
`~/.capsem/assets/manifest-metadata.json` with schema
`capsem.manifest_metadata.v1`; do not create a separate origin file. Runtime
adapters may derive an in-memory boot view. `GET /system/status` returns that
manifest, metadata, readiness, corporate state, and update comparison. CLI and
UI consume the same status contract; the UI must not synthesize publication
state.

Read `apple-signing.md` when touching signing, notarization,
certificates, Tauri keys, or Apple agreements. Read
`post-release-verification.md` after any public deployment.

## Published artifacts are load-bearing

A published manifest points at real storage, and some of it lives on GitHub
releases. Before deleting or retiring **any** published release, resolve what
the live manifests actually reference:

```bash
curl -s https://release.capsem.org/assets/<channel>/manifest.json \
  | grep -o 'releases/download/[^/]*' | sort -u
```

Check every manifest the catalog lists, not just `current` — a `supported` or
`deprecated` manifest keeps its own users alive. Both the VM assets **and** the
binary package can be hosted this way; assuming only one is a good way to break
the install path while believing you preserved it.

## Verify content, not status codes

**HTTP 200 is not proof that a resource exists.** The release site answers a
missing manifest with its SPA fallback: `200 OK` and an HTML body. Any check
that tests the status code passes while the manifest is absent.

Validate the bytes: parse the JSON, confirm the expected channel and version,
and verify the digest the catalog claims matches what was served.
`build_system/release_site/scripts/check-release-site-contract.py` does this and fetches every artifact a
manifest references, verifying size and sha256.

It runs at deploy time and, via `live-channel-watch.yaml`, daily and on demand.
The watch exists because the deploy gate can only notice a broken channel while
publishing a new one — anything that breaks an already-published channel from
outside a deploy (deleted release, artifact aged out by retention, CDN
misbehaviour) is otherwise invisible until the next release, and users meet it
first.

Run it by hand whenever you need to answer "is the channel healthy right now?":

```bash
uv run --project build_system --frozen python build_system/release_site/scripts/check-release-site-contract.py \
  --base-url https://release.capsem.org --channel stable --attempts 1
```

## Failure and retry discipline

- A red gate stops publication.
- Fix forward with a normal commit; never move a published tag or rewrite
  public release history.
- Do not blindly rerun unchanged work when the failure is deterministic.
- Preserve the previous public channel and installed working pair on any
  artifact, compatibility, tamper, test, or deployment failure.
- Treat disk, runtime, and runner capacity as tested release resources.
- Keep expensive artifact staging hardlink-first on the same-filesystem, with
  a tested cross-filesystem copy fallback and constrained-disk regression.
- Keep the clean-environment bootstrap proof before expensive work, while
  retaining the full installer E2E later.

### Diagnostic continuation is not release continuation

Use **diagnostic continuation** only to reach a late failure after a failed
non-release candidate. It may combine earlier outputs with current source:
zero means the segment passed, not that current source completed `just test`.
The transitional CLI spells this:

```bash
uv run --project build_system --frozen capsem-gate runs last --failed
uv run --project build_system --frozen capsem-gate candidate --prefix <retained-prefix> --from <failed-step>
```

Call it diagnostic continuation despite those legacy names. The named step
runs; predecessors are carried. Match prefix/frontier to the preceding failed
run, preserve its ID, and read `carried` as reused evidence, not a new `ok`.
Both release commands must reject these flags. Never use the result to stamp,
tag, push, dispatch, activate, or qualify. After the fix, rerun the public
release command from the beginning; only its clean complete plan may publish.

Read `ci-invariants.md` before editing release workflows. It carries
the platform, toolchain, scanner, disk, Docker, package, and runner lessons
learned from prior failures.

## Release-channel Cloudflare prerequisites

Before running a live binary or profile channel deploy, verify the Cloudflare
Pages project serving `release.capsem.org`, its `release.capsem.org` custom
domain, and both `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN`.
After deployment, run `build_system/release_site/scripts/check-release-site-contract.py`; it validates
BLAKE3/SHA-256 content, graph agreement, attestation references, and cache
headers rather than only checking that files exist.
