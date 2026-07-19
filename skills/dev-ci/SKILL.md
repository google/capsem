---
name: dev-ci
description: CI triage and red-gate response discipline. Use when any GitHub Actions run is red, when pr-gate blocks a PR, when deciding whether to rerun a failed workflow, when asked about CI health or history, or before merging anything while a gate is failing. Also use when a workflow file under .github/workflows/ is being edited. Covers stop-the-line policy, named-diagnosis-before-rerun, failure classification, and the job map of ci.yaml.
---

# CI Triage and Red-Gate Discipline

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
| `test-install` | ubuntu-24.04-arm | Docker install layout + systemd e2e | Dockerfile/install-script drift |
| `docs-build` / `site-build` / `release-site-build` | ubuntu-latest | Astro builds + release-site contract | pnpm lockfile drift; release-channel fixture drift |

Gotcha: Python suites in the macOS `test` job shell out to `pnpm --dir
release-site run build:channel` and friends. Those subprocesses need their
dependency installs done by earlier workflow steps -- `astro: command not
found` means a workflow install step is missing, not a test bug. The shared
web-surface script must fail immediately with a message naming the `Install
release site dependencies` step when `release-site/node_modules/.bin/astro`
is absent.

RustSec is blocking in scheduled/manual `security-audit.yaml`, not ordinary PR
CI. Treat a red security audit with the same named-owner discipline, while
keeping a newly published upstream advisory from invalidating every open PR.

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

## Release CI is different

Release evidence rules are stricter and live in `AGENTS.md` and
`/release-process`: only the successful remote `release-qualification.yaml`
run on the exact candidate SHA counts. A green local run, a nearby commit's
green run, or an agent's claim of passing tests is never release evidence.
Do not rerun the qualification gate for the same candidate; a failed
candidate gets forward fixes and a fresh qualification.

## Editing workflows

- `pr-gate` must list every job in `needs:` and test each result explicitly;
  a new job that isn't wired into `pr-gate` is not required and will be
  silently skipped by branch protection.
- Contract tests in `tests/capsem-release/` guard workflow invariants; run
  them after any workflow edit.
- Keep tool installs prebuilt/pinned; a workflow step that compiles tools
  from source on every run is a cost bug.
- Pin every external action to a full commit SHA and keep all
  `actions/upload-artifact` uses on one reviewed revision.
