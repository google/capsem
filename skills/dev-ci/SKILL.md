---
name: dev-ci
description: CI triage and red-gate discipline. Use when an Actions run is red, pr-gate blocks a PR, you are deciding whether to rerun, or you are editing .github/workflows/.
---

# CI Triage and Red-Gate Discipline

## Release input authority

The selected manifest is the bible: an artifact absent from it does not exist
for CI. Fetch mutable manifests fresh inside the owning serialized workflow.
Cache only immutable bytes under the digests recorded in that manifest, with
channel-independent cache identity, and verify every restored blob before any
gate consumes it.

## The law: stop the line

A red required gate stops the line. This is a mechanism, not a mood:

1. **No merging through red.** If `pr-gate` is red on any open PR against the
   same failure, nothing merges until the failure has a named diagnosis.
2. **No blind retries.** Every rerun must be preceded by a written diagnosis:
   which job, which step, root cause or explicit "suspected flake: <evidence>".
   Put it in the PR conversation or the commit message of the fix.
3. **One rerun per diagnosis.** If a "flake" fails twice, it is not a flake;
   treat it as a real defect and fix forward.
4. **Streaks are P0.** If the same job failed in 2+ consecutive runs, the gate
   has lost signal and repairing it outranks all feature work. A gate that is
   chronically red trains everyone to ignore it -- that is how a missing
   `pnpm install` once kept CI red for two weeks while work flowed around it.

Agents: these rules bind you absolutely. A rerun or merge without a diagnosis
is a protocol violation, not a judgment call.

## Map of the PR gate (ci.yaml)

Triggers: `pull_request` and pushes to `main`. The single required PR status is
**pr-gate**, which fans in these jobs -- all must be `success`. Superseded PR
runs are cancelled by PR-number concurrency; `main` runs are never cancelled,
so every merged commit retains a post-merge signal and Codecov baseline.

| Job | Runner | Covers | Common failure causes |
|-----|--------|--------|----------------------|
| `test-linux` | ubuntu-24.04-arm | KVM-backend unit tests + coverage | Linux-only cfg regressions; KVM absent is a warning, not a failure |
| `test` | macos-14 | Full Rust unit+integration, frontend, Python suites, schema drift, cross-compile check | Missing JS/Python dep installs for suites that shell out (see gotcha below); schema drift |
| `test-install` | ubuntu-24.04 | Docker install layout + systemd e2e (builds the x86_64 package) | Dockerfile/install-script drift; Linux bind-mount ownership, which macOS cannot reproduce |
| `docs-build` / `site-build` / `release-site-build` | ubuntu-latest | Astro builds + release-site contract | pnpm lockfile drift; release-channel fixture drift |

Gotcha: Python suites in the macOS `test` job shell out to `pnpm --dir
release-site run build:channel` and friends. Those subprocesses need their
dependency installs done by earlier workflow steps -- `astro: command not
found` means a workflow install step is missing, not a test bug. The shared
web-surface script must fail immediately with a message naming the `Install
release site dependencies` step when `build_system/release_site/node_modules/.bin/astro`
is absent.

That whole class is now mechanized, so it should never cost a CI round-trip
again: `tests/capsem-release/test_release_test_composition.py` asserts every
job in `ci.yaml`, `release.yaml`, and `release-assets.yaml` installs the tools
its own steps invoke, following justfile recipe dependencies transitively. It
runs in `_test-fast`, so the gap fails locally in seconds. Local `just test`
cannot catch provisioning drift any other way -- it runs where `just`, `pnpm`,
`node`, and `uv` are already on PATH, while CI provisions per job.

Its reachability deliberately over-approximates and does not model shell
branches. That bias is safe for "install this tool" and unsafe for "declare a
cache": `test-linux` reaches `_pnpm-install` statically but exits before it,
so declaring `cache: pnpm` there fails the post-job save with "Path(s) ... do
not exist". Provision generously; cache only where the store is observed.

The independently executable `_test-fast` module is the first local and CI
gate. It owns YAML/workflow and source syntax, source contracts, Clippy,
Python lint/type checks, JavaScript checks/builds, and the blocking Rust,
Python, and JavaScript vulnerability audits. `just fast-test`, local `just test`,
ordinary CI, and both release lanes call that exact module; workflows must not
reimplement or trim it. The scheduled/manual `security-audit.yaml` retains its
dedicated scanner schedule. A newly published advisory is a real red gate:
diagnose and remediate it or record an explicit reviewed exception in the
scanner's checked-in policy. Never turn the command into a warning or
`continue-on-error`.

## Triage procedure

```bash
# 1. Is this failure new, or a streak?
gh run list --workflow=ci.yaml --limit 10 \
  --json conclusion,displayTitle,createdAt

# 2. Which job failed?
gh run view <run-id> --json jobs \
  --jq '.jobs[] | "\(.conclusion)\t\(.name)"'

# 3. Which step, and why?
gh run view <run-id> --json jobs \
  --jq '.jobs[] | select(.name=="<job>") | .steps[] | select(.conclusion=="failure") | .name'
gh run view <run-id> --log-failed | grep -E "FAILED|error\[|AssertionError" -A 5

# 4. Failed runs upload test-artifacts (service.log, session.db, etc.)
gh run download <run-id> -n test-artifacts-macOS-1
```

## Classify before acting

- **Real regression** (the diff caused it): fix forward on the branch. Never
  rerun hoping it passes.
- **Environment drift** (new audit advisory, toolchain release, runner image
  change, external service): fix the environmental cause in its own commit
  with the diagnosis in the message. Never paper over it inside an unrelated
  PR, and never add an `allow`/skip without a written reason.
- **Infra flake** (runner died, network timeout, artifact upload hiccup):
  one rerun, after writing the diagnosis. Second failure = not a flake.

## Docker storage budget

`config/storage-policy.toml` governs the gate's Docker footprint. The numbers
are coupled and must stay satisfiable:

```
buildkit_keep_gib + minimum_free_gib + fixed usage  <=  minimum_disk_gib
```

Violate it and the floor can never be met by the one action taken to meet it —
`docker builder prune --keep-storage <keep>` cannot free space down to a floor
that sits above what it retains. The observable symptom is not "disk full": the
capacity probe *starts a container* to run `df`, so a thrashing daemon makes it
**time out**, and the gate dies reporting that it could not measure free space.

Fixed usage is the declared cache volumes plus base images (~18 GiB here).

**A too-small `buildkit_keep_gib` is a speed bug, not a safety margin.** At 24 GiB
against a ~35 GB hot graph, every pressure prune discarded layers about to be
reused and the host-builder image recompiled cold each run.

Age-based reclaim (`dangling-image-prune`, `buildkit-age-prune`, both 72h)
structurally cannot help during a burst of same-day runs — everything is younger
than the threshold. Expect `gc` to return near-zero after a heavy session; that
is the policy working, not failing. Provision headroom instead of pruning harder.

Thresholds are asserted in `build_system/tests/policy/test_docker_storage_policy.py`, so config and
contract move together.

## Release CI is orthogonal

Release rules live in root `RELEASE.md`; agent routing lives in `AGENTS.md` and
`/release-process`.

- `just test` is the complete local all-artifact proof.
- Binary CI builds packages only, pulls every selected profile, and runs the
  shared complete modules against that resolved pairing.
- Profile CI builds one channel/profile only, pulls the current package, and
  runs the same modules against that resolved pairing.
- Both entry workflows use the identical `capsem-release-${channel}` lock.
- No pairing becomes public without complete functional and glow-up proof.
- Diagnose and fix a failed lane forward; never bypass or selectively rerun
  away a failed required module.

The daily scheduler is orchestration, not a third release lane. It freezes the
event's full source commit, invokes
`just release-profile nightly <profile> <source-commit>` serially for every
selected profile, then invokes `just release-binaries nightly <source-commit>`
even if one profile command failed. Each command identifies and watches its own downstream run, whose
`capsem-release-nightly` lock owns the channel transaction. The separate
`capsem-nightly-release-scheduler` lock only prevents two daily orchestrators
from overlapping.

Nightly profile runs always rebuild assets; exact prior-run asset reuse is
stable-retry behavior. Nightly binary runs always rebuild and test native
packages. When the version tag already exists, CI disables publication after
the complete package and pairing gates because signed/notarized packages are
not reproducible bytes and immutable release assets may not be overwritten.
Stable has no scheduled trigger. Neither scheduler nor operators dispatch
`release.yaml` or `release-assets.yaml` directly.

## Editing workflows

- Every workflow step that invokes `just` is a declared enforcement edge in
  `build_system/tests/gate/test_exit_status_integrity.py`. The contract parses YAML and shell
  argv structurally: equivalent quoting, whitespace, comments, and line
  continuations remain green, while `continue-on-error`, `|| true`, disabled
  steps/jobs, command removal, and unclassified new `just` steps fail in the
  fast source-contract module. Extend that one inventory; do not add a local
  string-grep approximation.
- `pr-gate` must list every job in `needs:` and test each result explicitly;
  a new job that isn't wired into `pr-gate` is not required and will be
  silently skipped by branch protection.
- Contract tests in `tests/capsem-release/` guard workflow invariants; run
  them after any workflow edit.
- Keep tool installs prebuilt/pinned; a workflow step that compiles tools
  from source on every run is a cost bug.
- Pin every external action to a full commit SHA and keep all
  `actions/upload-artifact` uses on one reviewed revision.
