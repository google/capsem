# Qualification and Test Composition

Read this reference before changing either public release command, Python plan
composition, candidate/source guards, sandbox or egress behavior, fail-stop
ordering, local/release-CI test composition, complementary artifact staging,
or the paired `ProfileContent` boundary.

## Python owns release orchestration

The Justfile is only the public command and argv boundary: each release recipe
dispatches its arguments once to the matching `uv run --project build_system --frozen capsem-gate` subcommand.

Python under `src/capsem/gate/` owns the release graph:

- `candidateplan.py` composes reusable complete local verification;
  `release.py` declares the short source-validation and lane-dispatch graph.
- `qualificationevidence.py` validates exact complete and partial journal
  chains; `qualificationflow.py` is their one command-lifecycle seam.
- `qualification.py` parses one legal local/binary/profile state.
- `command.py` validates, inspects, locks, holds, records, and executes.
- Actions perform work, resources own lifecycle/evidence, and
  `config/gate.toml` owns values.

The release command does **not** launch or compose `just test`, Just, or
another `capsem-gate`. `just test <commit>` owns its local verification plan
under one process, lock, workspace, and run log. Release does not consume that
journal; its hosted lane qualifies the selected artifact family. A nested gate
still deadlocks on its parent's lock.

Read `/dev-gate` before changing this Python composition and `/dev-just` before changing its
public dispatch. Do not move orchestration into recipes, workflow YAML, or a release script.

## Hosted lanes own release qualification

The public release command forms are defined by root `RELEASE.md`:

```bash
just release-binaries <channel> <source-commit>
just release-profile <channel> <profile> <source-commit>
```

These are the sole publication entrypoints for humans and checked-in
automation. Each Python release plan has this non-negotiable order:

```text
just release-binaries <channel> <source-commit>
  1. require the detached source commit on fresh origin/main and fetch the
     serialized channel source manifest read-only; fail immediately if the
     manifest has no staged channel/profile authority
  2. publish the immutable source ref and dispatch the binary lane, which
     qualifies its exact package/profile pairing before publication

just release-profile <channel> <profile> <source-commit>
  1. require the detached source commit on fresh origin/main and publish its ref
  2. invoke capsem-admin and watch the exact self-qualifying profile workflow
```

The complete gate runs from an independent detached repository whose directory
name is the full commit. Only declared ignored signing input is copied in; Git
metadata and tracked source come from that commit. The outer checkout is not
the subject and may move while qualification runs. The source guard verifies
the frozen tree and records the commit in the run-start event.

Before its terminal event, every exact-source attempt hard-links its event
journal into the config-owned per-commit archive. Ordinary run rotation may
delete bulky step logs but cannot delete this qualification spine. A complete
event is emitted only after the whole candidate plan returns and the source
receipt binds the same commit and digest. A second `just test <commit>` either
records a normal lightweight success pointing to that archived run, or selects
the deepest graph-derived resume frontier supported by a retained full-SHA
prefix and an archived partial attempt. Resumed attempts name the exact parent
run and digest; recursive coverage of all carried ancestors is required.

Never infer release qualification from a local journal, skill, exit-code
memory, `latest`, marker file, or title-matching CI run. A candidate diagnostic
may resume only from its journal-derived prefix, frontier, and carried set.

Inside a step, a construction cache is a different mechanism. An asset or VM
builder may no-op only after hashing every authoritative source/config/tool
input and revalidating a live receipt over every retained output. The step
still runs and downstream assembly, integrity, boot, install, and glow-up proof
still executes. Do not describe that as a carried journal step, and do not let
an existence-only stamp authorize it. A valid warm receipt is a required hit;
running construction anyway is a failed reuse proof. VM assets are stored by
content identity and install-image receipts bind exact source, helper, Docker
runtime, platform, image ID/reference, size, and lifetime. Count, age, and byte
bounds use deterministic LRU, keep active/resumable selectors and receipts
pinned, and fail closed when those pins make a bound impossible. Nightly
profile release CI always rebuilds its selected profile; only a stable retry
may reuse the workflow's immutable completed artifact cohort.

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
manifest resolution, fresh remote-main validation, immutable source-ref
publication, and final dispatch actions.
Its one-time mode-0600 metadata is deleted before plan work; every brokered
command remains in the owning `GuardedRunner`, step log, and run journal. No
other candidate-module action may use `outside_sandbox=True`.

`just fast-test` remains explicitly incomplete developer feedback. It is the exact
private `_test-fast` module used by `just test` and release CI, including YAML
and source syntax, every source/release contract, Clippy, Python lint/type
checks, JavaScript checks/builds, and blocking Rust/Python/JavaScript
vulnerability audits. It is not release qualification; use a named
`just focus-test` group for targeted functional proof and the release commands
for publication qualification.

The selected version cohort must be prepared in the supplied commit before
qualification. Release-note/changelog organization is not a preflight: the
version tag—not a prewritten changelog heading—is the release event, and the
GitHub release records the exact source commit. The owning release command is
the first consequential publication command. Cheap read-only checks may precede it so a missing
serialized channel source, wrong-case paths, invalid workflow syntax, and
similar deterministic failures stop before hours of local work. The binary
preflight must fetch the mutable manifest/source
fresh and may not bootstrap profile state. If the staged channel/profile source
does not exist, the operator must use `release-profile` first. A failed local
diagnostic is still a release hold until understood, but it is not a journal
edge in the dispatcher. The dispatcher never changes
tracked files, commits, or pushes `main`. Test this fail-stop behavior by executing the public recipes with
fake downstream commands; inspecting recipe text alone is insufficient.

After that gate succeeds, both commands create or verify the lightweight
`capsem-source-<40hex>` tag at the exact commit and dispatch the workflow from
that tag while also passing `source_commit`. A different existing target,
malformed run identity, or workflow whose head SHA/ref changes is fatal.

Reusing recursively proven candidate work is not on this list. `auto` is the
candidate default: a carried step ran in the retained full-SHA prefix on this
source, its archived parent journal proves the ancestry, and the child records
it as `carried` rather than `ok`. Refusing that reuse cost four consecutive
160-minute qualifications of one release.

Do not extend candidate continuation authority to release attempts. Release CI
and the two public dispatch commands have no reusable journal for their short
release graph, so explicit `--from`, `--prefix`, and `--until` are refused.
Remote-main validation, mutable channel resolution, immutable source
publication, and final dispatch run fresh on every public attempt.

Do not introduce a skip flag, release-only reduced gate, preparation recipe,
environment-variable bypass, or direct checked-in caller of:

- `build_system/scripts/release/release-binaries.py`;
- `capsem-admin release` for a first-party public profile;
- `release.yaml` or `release-assets.yaml`.

Daily nightly automation snapshots `${{ github.sha }}`, calls
`just release-profile nightly <profile> ${{ github.sha }}` once for each
selected profile, then `just release-binaries nightly ${{ github.sha }}`. It never
dispatches either downstream workflow directly. Direct GitHub UI dispatch is
not the documented or tested release path.

Each command owns one artifact family. There is no combined release command.
The commands may run sequentially when a profile requires new code, but neither
may rebuild the other command's artifact family.

`config/public-surface.toml` locks this command surface. Treat any change as an
explicit product/API decision.

## Local proof and release-CI composition

Local `just test` is optional whole-world proof. Release commands neither run
nor require it: each hosted lane performs release qualification against the
manifest-selected complementary artifact family. The local and hosted paths
reuse the same checked-in private modules so test quality cannot drift.

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

Release automation uses the same public commands. The dispatched release
workflows save construction time, never test quality:

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

The shared module list is only half of parity. Release contracts must exercise
each legal execution envelope: ordinary checkout, exact detached dispatcher
prefix without a candidate receipt, and hosted qualification with materialized
inputs. A helper that branches on gate markers or release variables owes a
focused case for every legal branch; otherwise one test list describes several
incompatible "fast" realities.

Parse release workflows as YAML and pass each selected `run:` body to
`capsem.gate.shelllex`/`shellparse`. Never use regex, indentation slicing, or
`str.index` to infer jobs, steps, commands, assignments, redirections, or
heredocs. Presentation changes such as `echo` to `printf`, quoting, line wraps,
or YAML flow-to-block style do not change a release contract and must not spend
a hosted dispatch. Add the shared semantic reader first, then a Citadel guard
that prevents a second textual parser.

Assets and materialized configuration travel as one `ProfileContent` root.
Package construction, Debian proof, macOS Tart/physical-VZ proof, and final
install/glow-up must derive both paths from that one value and validate it
before Docker or Colima. Release CI stages raw manifest inputs into the paired
root on the host; the sealed proof never rematerializes them or falls back to
checkout `assets`/`cache/target/config` selectors.

Before public activation, the resulting pairing must pass manifest/artifact
integrity, every VM suite, Winterfell and MCP lifecycle, IronBank, injection,
integration, benchmarks, full `capsem-doctor`, native install, and update
glow-up. A staged incompatible profile may run only static, self-consistency,
integrity, isolation, and boot gates; the following binary lane must run the
complete functional and glow-up proof before activation.

The local gate records `HEAD` and a digest of all tracked and untracked
non-ignored source bytes. It supports ordinary uncommitted development and
fails if the source state changes while tests run.

Before dispatching a real release, use whatever focused or complete local proof
is useful for the change. `just test <source-commit>` is optional and never a
release prerequisite. Run the actual public release command, never a
hand-written workflow dispatch; it is the supported bridge into qualifying CI.

Complete local admission is impact-aware, but its proof is never partial. A
valid identical-source journal returns immediately. Otherwise unknown and
high-impact paths remain eligible, while explicitly low-impact paths under the
ten-commit cadence are refused with their exact `focus-test` owners. The
exceptional `just test <source-commit> force "<reason>"` records its reason
before work and cannot be used twice consecutively; only a successful normal
complete run resets it. None of this state authorizes release publication or
reuses a behavioral verdict across source identities.

## `--force`: the commit that is not the product

```bash
GITHUB_TOKEN="$(gh auth token)" \
  just release-binaries stable <commit> true  # the trailing `true` is --force
```

Both release commands need `GITHUB_TOKEN` in the environment. This machine keeps
the credential in `gh` rather than exported, so a shell that did not inherit it
runs the whole source proof and then dies at `channel-source` with "GITHUB_TOKEN
is required to resolve source manifests" -- four minutes in, naming the variable
but not where to get it.

Use it for **CI-only changes that do not affect local code or shipped bytes**: a
workflow file, a gate policy, a check that only ever runs on a hosted runner.
The artifacts such a commit produces are byte-identical to ones already
qualified, so re-proving them spends two and a half hours to learn nothing.
Paying that repeatedly is how a release stops happening at all -- the 0.6.0
binaries were held twice by a guard that only fired on disposable runners, and
each retry cost a full requalification of a product nobody had touched.

It is not a shortcut for product changes. Anything altering what ships --
crates, guest binaries, assets, profiles, packaging -- takes the full
qualification, because that run is the only thing that proves those bytes.

What it waives, and what it still records:

- The clean-worktree refusal is skipped, while Citadel and release-source
  contracts run before dispatch, so the tree still has to be one you
  would publish -- a release publishes the commit, never the working tree.
