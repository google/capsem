version: 1.6.1785062334
---
### Fixed

- Restored fail-closed release inputs and early gates: CI now caches
  manifest-selected native profile blobs by recorded digest, installed
  Doctor/Winterfell boots real assets with retained failure evidence, blocking
  audits/Clippy/web checks run before builders, all JavaScript workspaces audit
  clean, and the frontend uses its checked-in semantic theme without a Preline
  dependency.

### Added

- Added fail-closed release glow-up inputs that bind each orthogonal transition
  to its exact public-before manifest, candidate-after manifest, native package
  bytes, verified profile inputs, and selected profile publication.

- Added digest-verified candidate profile resolution that combines one locally
  staged immutable profile publication with unchanged manifest-hosted profiles
  without publishing or rebuilding either input.

- Added one cross-platform installed-transition evidence contract for fresh
  installs, orthogonal updates, staged profile-then-binary activation, channel
  switches, tamper rejection, full doctor, and Winterfell proofs.

- Added a fail-closed installed-Winterfell runner so persistence tests execute
  one exact installed binary, profile, and asset cohort without signing or
  falling back to source-built artifacts.

- Scheduled the nightly binary lane once daily through the same
  `just release-binaries nightly` command, with a clean no-op when `main`
  already points at an immutable binary release.

- Added `capsem-admin manifest corporate` for corporation-owned channel/profile
  manifests with exact or verified-latest official Capsem package selection,
  owned profile namespaces, and rejection of first-party or package writes.

### Changed

- Wired the binary release gate to the exact deployed public-before packages
  and profiles plus the authored candidate package, manifest, and complete
  profile cohort, classifying a single staged profile as a composed update.

- Wired the profile release gate to the exact deployed public-before package
  and profiles, staged and verified its immutable candidate publication once,
  and reused those same bytes for compatibility testing and publication.

- Allowed a composed binary release to validate and activate the complete set
  of previously staged profiles while still requiring every unchanged profile
  to remain byte-for-byte identical.

- Constructed hermetic Linux glow-up transports from the exact before/after
  native packages and verified profile cohorts, proving the local manifests
  differ from their manifest authorities only by artifact URLs.

- Moved binary candidate manifest authoring and host SBOM generation ahead of
  complete pairing tests, then reused that exact tested source manifest for
  immutable publication and final channel assembly without a second mutation.

- Enforced every active profile's minimum and maximum Capsem bounds before
  recording binary metadata or assembling a public channel, while preserving
  the exact staged profile bytes for later compatible-binary activation.
- Split profile authoring, pairing tests, and immutable publication so a
  profile requiring new code is built and staged once, remains absent from the
  public channel, and is later consumed unchanged by the compatible binary lane.
- Bound release compatibility tests to manifest-derived, digest-verified
  complementary artifacts; staged exact profile config and every selected
  profile image, replaced source-built host binaries with package inventory
  bytes, and made each selected channel profile an explicit VM-suite axis.
- Parameterized Winterfell, IronBank, doctor, MCP lifecycle, injection,
  integration, and benchmark execution across every active profile in the
  selected channel manifest without rebuilding either artifact family.
- Preserved every channel profile's membership, revision, config, evidence,
  and image identity in runtime update state instead of collapsing a public
  release graph to one default profile.
- Channel-qualified immutable profile config, image, inventory, and evidence
  paths so the same profile revision in different channels cannot alias bytes.
- Replaced the retired independent release gate with serialized orthogonal
  binary/profile lanes that reuse the unchanged artifact family, call the same
  complete test modules as local `just test`, and expose only
  `release-binaries` and `release-profile`.
- Centralized release-gate Docker capacity, cache retention, resource
  ownership, and debug-artifact limits in `config/storage-policy.toml`;
  retained a measured 24 GiB BuildKit cohort, released one-shot compiler
  outputs at their last consumer, recorded byte-accounted cleanup ledgers,
  and put both Docker and Tart working resources under the Python controller.

### Fixed

- Preserved installed doctor and failed VM-session evidence when the native
  package gate fails in CI.

- Bootstrapped an absent first-party channel through `capsem-admin release`
  with existing official packages and empty profile membership, while
  preserving non-selected public-channel manifests and profile bytes.

- Emitted the selected corporate channel identity in every validated
  `capsem-admin` corporate manifest.

- Passed the immutable release tag as an explicit binary-workflow input so the
  public release command cannot push a tag and then fail GitHub dispatch
  validation.

- Installed the pinned `uv` tool before the Linux install CI job enters the
  shared package gate.

- Retried transient Tart SSH failures only before authenticated guest
  execution, while failing without replay after a native package install has
  started.
- Removed stale source-contract references to the retired asset-delta helper
  so the orthogonal release doctrine teardown remains internally executable.
