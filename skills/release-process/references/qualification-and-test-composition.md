# Qualification and Test Composition

Read this reference before changing either public release command, Python plan
composition, candidate/source guards, sandbox or egress behavior, fail-stop
ordering, local/release-CI test composition, complementary artifact staging,
or the paired `ProfileContent` boundary.

## Python owns release orchestration

The Justfile is only the public command and argv boundary: each release recipe
dispatches its arguments once to the matching `uv run capsem-gate` subcommand.

Python under `src/capsem/gate/` owns the release graph:

- `release.py` declares commands/publication edges; `candidateplan.py` composes
  the complete test fragments.
- `qualification.py` parses one legal local/binary/profile state.
- `command.py` validates, inspects, locks, holds, records, and executes.
- Actions perform work, resources own lifecycle/evidence, and
  `config/gate.toml` owns values.

The release command does **not** launch `just test`, Just, or another
`capsem-gate`. It composes the exact candidate plan under one process, lock,
workspace, and run log. A nested gate deadlocks on its parent's lock; a split
design needs the parallel receipt authority the manifest contract forbids.

Read `/dev-gate` before changing this Python composition and `/dev-just` before changing its
public dispatch. Do not move orchestration into recipes, workflow YAML, or a release script.

## One command owns the complete release

Capsem has exactly two release-facing Just commands:

```bash
just release-binaries <channel>
just release-profile <channel> <profile>
```

These are the sole release entrypoints for humans and checked-in automation.
Do not ask an operator to run a preparation command or a separate `just test`
first. Each Python release plan contains the complete plan used by `just test`
and has this non-negotiable order:

```text
just release-binaries <channel>
  1. validate release notes and fetch the fresh serialized channel source
     manifest read-only; fail immediately if the manifest has no staged
     channel/profile authority
  2. compose and execute the complete `just test` candidate plan in-process
  3. only after success: run the binary release script and dispatch binary CI

just release-profile <channel> <profile>
  1. compose and execute the complete `just test` candidate plan in-process
  2. only after success: invoke capsem-admin release for that channel/profile
  3. correlate and watch that exact profile workflow through terminal success
```

The complete gate runs from a private source generation. Prechecks, source-head
capture/reconfirmation, and publication target the originating checkout because
work authored only in the disposable copy would disappear. The source guard
stops publication if the checkout no longer has the recorded HEAD and bytes.

The complete executor is also kernel-isolated for the entire candidate graph:
Bubblewrap provides a loopback-only namespace on Linux and Seatbelt provides
the macOS boundary. Network access is not restored process-wide for release.
Candidate and both release commands refuse explicit `--sandbox off` and
`--sandbox report` before constructing a plan, re-executing, or acquiring a
resource. Report-mode measurement remains available only on directly invoked
incomplete modules, so permissive evidence cannot be mistaken for complete
qualification.
An authenticated helper created immediately before sandbox re-exec serves only
the explicitly marked RustSec, npm bulk, and OSV advisory queries plus
manifest-resolution, exact-main confirmation/push, and final dispatch actions.
Its one-time mode-0600 metadata is deleted before plan work; every brokered
command remains in the owning `GuardedRunner`, step log, and run journal. No
other candidate-module action may use `outside_sandbox=True`.

`just fast-test` remains useful public developer feedback. It *is* the exact
private `_test-fast` module used by `just test` and release CI, including YAML
and source syntax, every source/release contract, Clippy, Python lint/type
checks, JavaScript checks/builds, and blocking Rust/Python/JavaScript
vulnerability audits. It is still not release qualification and must never
replace `just test` in either release command.

`just test` must be the first consequential command. Cheap read-only checks may
precede it so missing notes, a missing serialized channel source, wrong-case
paths, invalid workflow syntax, and similar deterministic failures stop before
hours of local work. The binary preflight must fetch the mutable manifest/source
fresh and may not bootstrap profile state. If the staged channel/profile source
does not exist, the operator must use `release-profile` first. If `just test`
fails, the release command must stop before stamping versions, changing tracked
files, committing, tagging, pushing, authoring a shared manifest, or dispatching
a workflow. Test this fail-stop behavior by executing the public recipes with
fake downstream commands; inspecting recipe text alone is insufficient.

After that gate succeeds, both commands run the same checked-in source guard.
It requires the clean `main` HEAD captured before `just test`, then
fast-forward-pushes that exact tested HEAD when it is ahead of `origin/main`.
It refuses a changed HEAD, dirty tree, divergence, or force-push. Only after
this guard may binary stamping or profile dispatch begin.

Do not introduce a skip flag, release-only reduced gate, preparation recipe,
environment-variable bypass, or direct checked-in caller of:

- `scripts/release-binaries.py`;
- `capsem-admin release` for a first-party public profile;
- `release.yaml` or `release-assets.yaml`.

Daily nightly automation calls `just release-profile nightly <profile>` once
for each selected profile and then `just release-binaries nightly`. It never
dispatches either downstream workflow directly. Direct GitHub UI dispatch is
not the documented or tested release path.

Each command owns one artifact family. There is no combined release command.
The commands may run sequentially when a profile requires new code, but neither
may rebuild the other command's artifact family.

`config/public-surface.toml` locks this command surface. Treat any change as an
explicit product/API decision.

## Local proof and release-CI composition

Local `just test` is the whole-world proof. Release commands run it in full
before any release side effect, then CI reuses the same private modules against
the manifest-selected complementary artifact family.

`just test` is the complete local CI-equivalent proof, not a smaller developer
smoke test. Before any Docker/Colima, bootstrap, package, profile, asset, or VM
work, it runs the independently executable `_test-fast` module. It then
rebuilds every package and every checked-in profile and runs all six checked-in
modules:

- `_test-fast`
- `_test-static`
- `_test-artifacts`
- `_test-functional`
- `_test-glowup`
- `_test-release-contracts`

Every test, scanner, contract, build validation, and tool dependency required
by release CI must be reachable from this command. A gate that exists only as
inline workflow YAML is a parity defect until it is extracted into a
checked-in module called by `just test`. Each module must own its prerequisites
and must also be executable independently in a clean local environment. Never
rely on a package installed incidentally by an earlier workflow job or by a
developer machine.

The cheap failures run before VM and artifact work. They include formatting,
lint, Rust clippy, Python checks, JavaScript/frontend checks, action/workflow
validation, source contracts, and vulnerable-dependency audits for every
locked ecosystem. The complete proof still includes all expensive gates:
artifact validation and boot, every VM suite, Winterfell, MCP lifecycle,
IronBank, injection, integration, benchmarks, full `capsem-doctor`, native
package installation, and glow-up transitions. None is advisory.

Release automation uses the same public command and therefore receives the
same complete `just test` gate before dispatch. The dispatched release
workflows then save construction time, never test quality:

- the binary lane builds packages only and resolves every selected-channel
  profile by manifest-recorded digest;
- the profile lane builds exactly one channel/profile and resolves the
  selected channel's current package by manifest-recorded digest;
- both lanes stage those exact resolved artifacts into the same test modules
  used locally;
- source-built substitutes must not replace the resolved complementary family.

The workflow must download and digest-check those immutable inputs and
materialize every locked language dependency before calling `_test-*`.
Directly invoked modules enter the same host-kernel sandbox themselves; a cold
runner discovering a missing dependency after entry is a workflow preparation
bug, not permission to fetch during qualification.

This is one test architecture with two artifact-preparation modes, not a local
test path and a forked CI test path. The test modules must not silently choose
different assertions based on ambient release environment variables. Artifact
preparation may differ—local builds both families, a release lane downloads the
unchanged family—but the resulting manifest-addressed bundle enters the same
module implementations.

Assets and materialized configuration travel as one `ProfileContent` root.
Package construction, Debian proof, macOS Tart/physical-VZ proof, and final
install/glow-up must derive both paths from that one value and validate it
before Docker or Colima. Release CI stages raw manifest inputs into the paired
root on the host; the sealed proof never rematerializes them or falls back to
checkout `assets`/`target/config` selectors.

Before public activation, the resulting pairing must pass manifest/artifact
integrity, every VM suite, Winterfell and MCP lifecycle, IronBank, injection,
integration, benchmarks, full `capsem-doctor`, native install, and update
glow-up. A staged incompatible profile may run only static, self-consistency,
integrity, isolation, and boot gates; the following binary lane must run the
complete functional and glow-up proof before activation.

The local gate records `HEAD` and a digest of all tracked and untracked
non-ignored source bytes. It supports ordinary uncommitted development and
fails if the source state changes while tests run.

Before dispatching a real release, run the actual public release command, not
`just test` followed by a hand-written dispatch. Its embedded `just test` is
the local proof and its remaining steps are the only supported bridge into CI.
Do not dispatch CI until that embedded local proof completes successfully.
