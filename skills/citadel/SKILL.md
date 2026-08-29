---
name: citadel
description: The Citadel guard suite. Use when adding a guard, a linter, or a source surface, or when a citadel test fails and you need to know what it is protecting.
---

# The Citadel

`tests/citadel/` is where Capsem records architectural mistakes that must not
be repeated. Guards are source-level, run in the fast phase, and each one
carries its reasoning in the failure message.

The organising idea is **shape**. Every guard here says: constrain the form so
a property stays provable. A recipe is a dispatch. A module has a ceiling. A
shell body is a sequence of simple commands. An enforcement line is the whole
command. When shape lapses, something grows past the point where a tool, a
reviewer or a test can take it in, and then nothing checks it at all.

## Why it runs first

Every guard reads source and asserts on it. None needs an artifact, a VM or a
daemon. They were reachable only through the broad suite's `root`, which
carries `require_artifacts` and runs after the whole asset build -- so a
DB-boundary violation surfaced once the VMs were up, roughly forty minutes
after the source that caused it was read.

`fast.citadel` now runs beside Ruff. `tests/citadel` is in `broad_ignores`, so
the suite has one collector and its failures have one owner.
`test_guard_scheduling.py` fails if it is moved back.

## The guards

| guard | what it holds |
|---|---|
| `test_db_boundary.py` | only `capsem-logger` executes ledger queries |
| `test_lint_coverage.py` | every declared surface has a checker that exists and runs early |
| `test_shape_boundaries.py` | every `[boundary.*]` ceiling and its exact debt inventory |
| `test_workflow_enforcement.py` | a gating step cannot pass while failing |
| `test_container_workspace.py` | the guest builder's `/src/*` glob skips dotfiles |
| `test_cross_platform_tool_lock.py` | platform-specific Python tools stay on one reviewed artifact cohort |
| `test_release_test_environment.py` | nested tests cannot inherit their parent release's source identity |
| `test_skill_context_budget.py` | the skill description budget every session pays |
| `test_hot_build_contract.py` | hot codecs stay optimized in the dev profile |
| `test_package_architecture_boundary.py` | package and machine architecture never cross |
| `test_guard_scheduling.py` | the Citadel itself runs before the expensive work |
| `test_run_digest_echo.py` | the cross-run digest reaches whoever works next |
| `test_tree_copy_boundary.py` | only `filesystem` copies a tree, and never through a link |
| `test_rust_check_coverage.py` | clippy, nextest and doctests between them cover all Rust |
| `test_agent_contract_is_one_file.py` | every agent is held to the same contract |
| `test_step_attributes.py` | a step says what it is; nothing infers it from a label |
| `test_work_graph_invariants.py` | the plan's graph properties, asked of the graph |
| `test_workflow_script_checkout.py` | a workflow checks out its source before running a checked-in script |

## Adding a guard

Three rules, each learned by getting it wrong.

**State the reason in a named `*_RATIONALE`, appended to the assertion.** A
violation must teach, because the reader is usually someone who does not
already know. `test_db_boundary.py` is the model. Where the reason is already
stated canonically -- `config/gate.toml`, AGENTS.md, a skill -- cite it rather
than restate it: a second wording is a second thing to keep in step.

**Write the adversarial case, not just the conformance case.** A guard proved
only by inputs it was built for is not proved. `test_workflow_enforcement.py`
enumerated ways to neutralise an enforcement check and an adversarial pass got
five past it -- `; :`, a trailing `&`, `| cat`, `set +ex`, `set +o errexit`.
The rule is now a whitelist for that reason: the comparison must be the whole
command, so any token that could consume its exit status fails, predicted or
not. Enumeration cannot be completed; inversion is complete by construction.

**Break it once and watch it go red.** A guard built from the current state
asserts nothing until it has been seen failing.

## Adding a surface

`[[lint_surfaces]]` is the map: one entry per kind of first-party file, naming
every step that must check it. It is the answer to "what checks what", so read
it before grepping for a checker.

Two fields, because two different promises. `enforced_by` must exist *and*
answer in the fast phase -- that is for checks needing no build, where running
late means reporting after the expensive work they should have preceded.
`checked_by` must exist in any phase, for checks that legitimately cannot be
early: the Rust test runners need a compiled workspace, so they sit beside the
coverage run rather than beside Ruff. Late is acceptable; missing is not, and
before `checked_by` existed the inventory recorded which surfaces were *linted*
while saying nothing about which were *tested*.

`test_lint_coverage.py` proves the surface has files, that every named step
exists in the plan, and that the `enforced_by` ones run early.

Shell went unchecked across 6,821 lines while four `# shellcheck disable=`
directives sat in the tree, written for a linter no lane ran. Markdown was
79,000 lines with nothing pointed at it. Neither was decided; both surfaces
arrived without a gate and nothing was watching for the gap. Adding a language
now means adding its gate in the same change, or the Citadel fails.

## The lint harness

`build_system/scripts/audit/lint_harness.py` is the one spelling of "run a linter": a `Tool`
declares argv and how to read its output, `Sources` yields `(name, text)`
however it must, and every finding renders identically. `build_system/scripts/audit/check-surfaces.py`
is the entry point, one surface per invocation.

`Sources` exists because the shapes differ. `*.sh` is a file. A workflow `run:`
body lives inside YAML. A Dockerfile `RUN` body lives inside an instruction,
and for a `.j2` template it does not exist until rendered -- so it is rendered
through `render_dockerfile`, the same entry point the image build uses. A
masked template is a fiction, and linting a fiction reports on a file nobody
builds.

`capsem.gate.shellsurfaces` owns extraction, shared by the linter and the shape
guard so they cannot disagree about what a body is. Getting it right took five
corrections, and each is worth knowing before adding a sixth surface:

- flattening `\` continuations pulls an inline `#` into the middle of a logical
  line and comments out the rest
- comments must be stripped *before* continuations are joined, the order Docker
  uses; reversing it leaves a dangling `\`
- `${{ }}` masks to a variable reference, not a literal -- a literal turns real
  comparisons constant and produces phantom `SC2050` findings
- Jinja is rendered, never masked
- workflow bodies are keyed by index as well as name, because step names are
  not unique within a job and a collision silently dropped five of 184 steps

**Every surface fails closed.** A surface that yields no sources raises
`EmptySurface`. "Found nothing so it was skipped" is how a gate stops being
one.

## The run digest

A guard catches a mistake at the moment it is made. The digest catches the
class a guard cannot: the ones only visible across runs -- a step that fails
one time in four, a phase that doubled three days ago, a critical path made of
queueing. Each of those looks like bad luck in the single run anybody looks at.

`target/gate-runs/DIGEST.md` is regenerated by `fast.digest` and again at
`RunLog.close`, computed from `target/gate-runs/ledger.jsonl` -- one distilled
row per finished run, kept because the run directories are not (`keep_runs` is
twenty). A session-start hook prints it, and all three agent contracts name it,
because only one agent gets the hook.

Three questions, three scopes, and conflating them is the mistake worth
knowing about. A **duration** may only be compared against an identical plan
shape, argv and host class -- `runledger.identity`, the same rule the release
ratchet uses. A **critical path** needs the same command. A **failure count**
needs neither, and scoping it like the other two hid a step that had failed
three times that week behind one unrelated green fast-test run.

An empty baseline says so. "Nothing anomalous" when nothing was compared is
the one thing this document must never print.

## Debt inventories

Every ceiling and every known-missing target is an exact inventory, not an
exemption list. An entry may not grow. A file that shrinks updates or removes
its entry in the same change -- an inventory that drifts from the tree has
stopped ratcheting. `[boundary.scripts]`, `[boundary.rust]`,
`[boundary.shell_bodies]` and `[lint.markdown]` all use this shape.

A ceiling is an outlier detector, not a rewrite mandate, so it comes from the
tree's own distribution. Rust's median tracked file is 232 lines; borrowing
Python's 300 would flag 43% of the tree and be deleted the first time it
blocked someone.

## See also

`/dev-gate` for the gate the guards run inside, `/dev-testing` for where a new
test belongs, `/ironbank` for black-box acceptance proof.
