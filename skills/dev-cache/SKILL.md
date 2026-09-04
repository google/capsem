---
name: dev-cache
description: Capsem's unified cache control plane. Use for cache inventory, retention, cleanup, test reuse, or disk, Docker/Colima, Tart, Cargo, Python, Node, BuildKit, and VM asset cache work.
---

# Cache Control

## One authority

`config/cache.toml` is the only cache inventory and policy. Every independently
accounted cache has exactly this common contract:

- `description`: what expensive work the bytes preserve;
- `scope`: `disk`, `docker`, or `tart`;
- `max_size_bytes`: the usage ceiling that triggers enforcement;
- `warm_size_bytes`: the retained target after crossing the ceiling;
- `prune_strategy`: `lru`, `generational`, `ephemeral`, `none`, `docker`, or
  `tart`.

Age and count fields are optional refinements. They never replace the common
contract. Do not introduce storage-availability rails, warning thresholds, or
a second config table. Cache decisions are based exclusively on owned usage.

## Typed boundary

Programs use `CacheRegistry`, `CacheRequest`, `CacheOperation`, and
`CacheMutationResult` from `capsem_builder.cache`. They select a stable cache ID
and do not branch on storage mechanism or discover backend locations.

The registry owns backend dispatch:

- `DiskBackend` inventories repository cache stages through `CachePaths`;
- `RuntimeBackend` inventories and mutates Docker or Tart through their native
  adapters;
- Docker-on-Colima is still the `docker` cache. Colima is an implementation
  detail of that backend, never a separately billed cache;
- Docker image repositories are child cache IDs but use the same contract and
  request/result types as every other owner.

A producer that must place disk bytes asks `CachePaths` or the gate's
`cachelayout.stage_path()` for its configured stage. It must not reconstruct a
path from repository literals, environment variables, or a backend name. A
consumer that only inspects or controls a cache uses `CacheRegistry` and never
receives its path.

Pinned external executables use `CachedToolPolicy` and `materialize()`. The
policy declares one HTTPS URL and SHA-256 per host platform; the primitive
downloads once, verifies before atomic publication, installs mode 0555, and
returns a typed hit/miss result. A subsystem must not invent its own downloader
or tool directory.

Short-lived results from mutable services use `CleanVerdict`, `reusable()`, and
`record_clean()`. The subject digest covers the complete typed policy and exact
input bytes. Only success is recorded; malformed, future-dated, expired, or
non-matching receipts are misses. The owning cache stage supplies the maximum
age, count, warm size, and maximum size.

`CachePaths` is loaded through `load_paths()`. The loader resolves the
policy-owned `authority_environment`; a gate prefix exports that variable once
so every producer sees the outer shared cache even when its output path crosses
a prefix symlink. Producers never read or interpret the environment variable
themselves.

With no explicit authority, `load_paths()` resolves a linked Git worktree to
the checkout that owns the common `.git` directory. All branches of one local
repository therefore share one cache inventory. Do not create a cache beside
each worktree or pass a worktree path as implicit storage. Administrative
cleanup of a retired location uses the typed CLI's explicit `--repository`
with `--policy-repository`; ordinary operators stay on `just cache`.

All models are strict, frozen Pydantic models with `extra="forbid"`. Keep every
module at or below 300 lines and put tests in the matching test module.

## Operator interface

The public entry point is `just cache`. These are the complete supported
inspection and lifecycle commands:

```bash
just cache stats                 # usage, warm/max contract, state, description
just cache stats --json          # typed machine-readable inventory
just cache stats --offline       # disk inventory without native runtime calls
just cache contract <cache-id>   # the five-field common contract
just cache verify                # containment and complete disk accounting
just cache prune [cache-id]      # deterministic preview; defaults to all
just cache prune <id> --apply --reason "why"
just cache enforce <cache-id> --reason "why"
just cache clean <cache-id>      # explicit cold-clean preview
just cache clean all --apply --reason "why"
```

`prune` follows ordinary age/count policy and only recovers to `warm_size_bytes`
after usage crosses `max_size_bytes`. `enforce` is an applied preflight and
fails if protected state prevents compliance. `clean` is the explicit cold
operation; it still preserves active leases and protected generations.

Retained cache lifetime ends only through these typed operations. Do not add
consumer-boundary releases, post-test eviction hooks, or other subsystem
lifecycle paths that bypass the owner's warm/max/age/count policy.

Every applied mutation is exact, ownership-scoped, and journaled. Do not use
broad `docker system prune`, direct Tart deletion, `rm` over cache roots, or
backend-specific cache CLI commands. Foreign Docker/Tart resources are visible
only as native totals and are never deleted by Capsem.

Repository-generation reclaim is anchored by an exact current tag. Docker's
bulk image listing can briefly lag a completed BuildKit import, so the Docker
adapter verifies that exact tag directly and supplies a protected typed
resource to the planner. Never turn this into an unanchored retry or allow a
missing exact inspection to prune older generations.

## Test efficiency

Start with the cheapest evidence that owns the change:

```bash
just fast-test
just focus-test <owner>
```

`just test <source-commit>` is the optional complete local proof. Its journal
reuses an exact successful source identity, and its graph carries valid
artifact frontiers instead of rebuilding them. Low-impact paths are routed by
`[test_admission]`; forcing complete proof repeatedly is rate-limited by
commit distance. Release commands self-qualify and do not consume a local
`just test` prerequisite.

Working-tree gates derive their private prefix name from the exact source
digest. Do not replace it with a random run ID: Cargo fingerprints contain
absolute source paths, so random prefixes turn an unchanged repeat into a
rebuild. The gate owns sccache as a scoped `CompilerCache` resource, exports
`SCCACHE_BASEDIRS` (plural), uses client-side mode, and stops the server during
resource teardown. Do not manage its daemon in shell or disable Cargo
incremental compilation without a measured workload-specific reason.

Never make normal cache reuse opt-in. Do not clean caches to diagnose a product
failure unless cold-state behavior is itself the subject under test.

## Changing policy

1. Run `just cache stats --json` and preserve the before inventory.
2. Change the owning cache entry in `config/cache.toml`.
3. Update Pydantic validation and focused cache tests together.
4. Run `just cache verify` and the cache, gate, and Citadel suites.
5. For a size change, run one complete gate with permissive maxima, record the
   post-run inventory, and derive warm/max values from observed retained work
   plus measured generation needs. Validate the ratchet with an exact-repeat
   run so it proves reuse rather than merely fitting a cold build.

Do not infer a cache limit from filesystem capacity. Machine provisioning and
owned cache retention are separate concerns.

## Debugging

Read `cache/target/gate-runs/DIGEST.md` before reporting gate state. Then use:

```bash
uv run --project build_system --frozen capsem-gate runs last --failed
uv run --project build_system --frozen capsem-gate runs trend --step <label>
just cache stats --json
just cache contract <cache-id>
```

If runtime inventory fails, diagnose Docker/Colima or Tart through `/dev-setup`.
Do not add fallback accounting or silently treat a required runtime as empty.
