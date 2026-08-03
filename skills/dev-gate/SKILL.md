---
name: dev-gate
description: How capsem-gate works and how to add or change a gate command. Use when touching build, test, or release logic, or when a boundary/primitive/recursion/purity guard fails.
---

# The build and release gate

The justfile dispatches; `src/capsem/gate/` decides. No recipe carries a shell
body, none exceeds five lines, and both are contract tests rather than
conventions.

`just test` is **one process, one machine lock, one workspace, one plan** — 64
steps and 91 actions in a single graph. Both release commands *contain* that
same plan rather than launching it.

## The rule everything else follows from

**A plan action may never invoke `just` or another `capsem-gate` command.**

`GuardedRunner` refuses it at runtime, seeing through `uv run` and `caffeinate
env`. This is not style: the machine lock is not reentrant, so every such call
was a child waiting out its 7200-second timeout for the lock its own parent
held. Twenty-two of them existed, and each read perfectly at the call site —
`Run(["just", "_sign"])` looks like naming a step.

When you need another command's work, **compose its fragment**:

```python
def fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """Add this module's steps to `plan`; return its terminal step."""
```

Groundwork several fragments share uses `plan.shared(...)`, which makes the
second caller a dependant rather than a duplicate. **Pass `after` to the work,
not to the shared step** — sequencing shared groundwork behind one of its
consumers is a cycle, and one that only appears once two lanes compose.

A fragment that has more than one leaf returns *all* of them. `static` returned
only its last, so the phases after it started while a storage release it owned
was still outstanding.

## The interpreter comes first

`capsem-gate` is `capsem.gatelaunch:main`, not `capsem.gate.cli:main`. It
re-execs under a per-invocation `pycache_prefix` **before importing any of
`capsem.gate`**, and exports `PYTHONPYCACHEPREFIX` so pytest and every other
child inherits it. CPython validates a `.pyc` by mtime and size, so two
same-length edits inside one timestamp tick leave stale bytecode that still
looks current — which produced 74 false failures during one review, and which
would otherwise let a release qualify code that is not in the tree.

The complete gate refuses in `source.record` if `CAPSEM_GATE_PYCACHE` is
absent. If you drive a full plan from a test, set it.

Nothing in `gatelaunch` may import `capsem.gate` at module scope.

## Release state is one indivisible value

`Qualification.from_environment` is read once in `GateCommand.__init__` and
passed to `artifacts()`, `functional()` and `glowup()`. Three legal shapes —
local, binary release, profile release — and every partial combination is
refused while the plan is being built. **No module below reads
`CAPSEM_RELEASE_*` itself.** They each did, from a different variable, and a
dropped `GITHUB_ENV` line built a plan that verified pulled assets against a
source-rebuilt package.

In a test, hand the command an explicit `qualification=` rather than exporting
a variable.

## `execute()` enforces; you inherit it

Never overridden — a contract test fails if a subclass defines it. In order:

1. `plan()` is built with the machine **sealed** (`planseal.sealed()`). Ambient,
   not per-runner: `release.py` escaped an instance-scoped seal by constructing
   its own `Runner` inside `plan()`.
2. `plan.validate(config)` — cycles, declared exclusives, one owner per
   artifact. Before the lock, so a bad plan costs nothing.
3. `--graph` / `--dry-run` answered. **Before** `reexec()`, or asking becomes
   doing.
4. `reexec()`, outside the lock.
5. `RunLog.open` → `GuardedRunner` → `held(*resources)` → `plan.run(context)`.
6. `_summarize` — outside the log's context, so `run.end` is on disk first.

## Declaring a command

```python
class MyCommand(GateCommand, name="my-command", help="one line for --help"):
    exclusive = True   # default is False; anything that WRITES needs True
    records = True     # False only for commands that read runs

    def resources(self):        # acquired in order, released in reverse
        return (Workspace(self._config),)

    def plan(self) -> Plan:
        plan = Plan(self.name)
        fragment(plan, self._config)
        return plan
```

Add the module to `COMMAND_MODULES` in `cli.py`.

**`exclusive` is about cross-process safety.** `[execution.exclusives]` entries
are `threading.Lock`s: they order steps inside one plan and coordinate nothing
between two `capsem-gate` processes. `just _sign` in one terminal could replace
the codesigned binaries a qualification in another was executing.

## Step or resource?

The question is *when it must happen*.

| | |
|---|---|
| **Step** | Work. Skipped when its dependency fails — which is right: it was written against something that was never produced. |
| **Resource** | Anything that must happen on **every** path including the aborted one. `held` releases in reverse; `preserve` runs on failure *before* release, because release destroys the evidence. |

The orphan-process count, the Colima lifecycle and the failure-evidence capture
are resources. The source-state check is a pair of steps — it must *not* run
when the gate failed, because the failure is the report.

## Everything is data

Every path, filename, architecture, channel **and environment variable name**
comes from `config/gate.toml`. `tests/test_gate_has_no_literal_data.py` catches
literals on both sides — a name read *and* a name used as a key in an
environment you hand to a process. Standard conventions (`HOME`, `TMPDIR`,
`PKG_CONFIG_PATH`, `HOST_UID`) are allowlisted; Capsem rails are not.

Reach for the builders rather than assembling dictionaries:
`config.environment.capsem(home=…, run_dir=…)`,
`config.environment.content(assets=…, profiles=…)`, and the typed families
under `[environment.package]`, `[environment.release_site]`,
`[environment.install_proof]`.

## Secrets are declared on the invocation

A `Command` that names credentials in `secret_env` cannot render them — not
through `str()`, not in the journal, not in the exception a failure raises. The
name survives, the value becomes `<redacted>`, and `evidence_argv` scrubs the
value out of argv too. Docker gets `-e NAME` and takes the value from its own
environment, because argv is readable through `ps` and no log filter covers
that.

## Every `Call` says why it is one

`Call` renders as prose, so a plan built from them says less. Pass
`why=Why.SECRETS` (one phase earns this: the package build), `Why.DYNAMIC`
(argv known only at run time) or `Why.COMPUTATION` — the last is a to-do,
not a design, and usually wants to become a named action.

## Stopping a run

`cancellation.check("what you are doing")` at points a partial unit can be
abandoned from — between files, between chunks, never mid-write. Ctrl-C sets
the switch, cancels pending steps, wakes lock waiters, and waits a bounded ten
seconds before naming whatever refused to stop. Pool workers do **not** inherit
the submitting context: `planrunner` hands the switch to `_guarded` explicitly.

## Asking without running

```bash
uv run capsem-gate <command> --dry-run    # every step, every action, real argv
```
```bash
uv run capsem-gate <command> --graph      # the same graph as mermaid
```
```bash
uv run capsem-gate runs last --failed     # what broke, where, how long
```
```bash
uv run capsem-gate gc --dry-run           # what disk the gate holds, per tree
```

`runs` and `gc` do not record themselves: `runs last` used to open a run and
repoint `latest` at the question.

## The guards that will fail you

| Test | What it holds |
|---|---|
| `test_gate_execute_funnel.py` | recursion refused; every subprocess logged; plan construction inert; isolation from acquired resources |
| `test_gate_no_nested_commands.py` | the same recursion rule statically, plus every named recipe and subcommand resolves |
| `test_gate_boundary.py` | no shell bodies; ≤5 recipe lines; ≤300 module lines; `ty` strict |
| `test_gate_primitives_are_the_only_way.py` | only the harness touches the machine; only `planrunner` schedules |
| `test_gate_has_no_literal_data.py` | no path, architecture or channel spelled in code |
| `test_gate_hardening.py` | mutation is exclusive; plans are pure; verifications ask the real question |
| `test_gate_runlog_evidence.py` | attribution under concurrency; run status; non-recording inspection |
| `test_gate_lifecycle.py` | acquire order, reverse release, preserve first, primary error survives cleanup |
| `test_gate_qualification.py` | the three legal release states; every partial one refused |
| `test_gate_secrets.py` | no signing material in argv, journal, summaries or errors |
| `test_gate_source_identity.py` | the launcher; stale bytecode cannot be qualified |
| `test_gate_step_output.py` | each step keeps what its commands printed |
| `test_gate_cancellation.py` | Ctrl-C stops pending, running and waiting work |
| `test_just_argument_boundary.py` | every recipe parameter crosses one exact argv boundary |

## Testing a command

`tests/helpers/gate.py` is the one place that knows how to interrogate the
gate. Assert **edges**, not positions.

| | |
|---|---|
| `RecordingRunner` | records what a plan would run, for a test that drives its own plan |
| `gate_plan(name)` | a built plan; `after_of(label)` is how "these run in parallel" is asserted |
| `gate_labels(name)` | its step labels in graph order, for ordering claims |
| `gate_issues(name)` | real argv for everything a command would issue -- `None` reads the whole gate |

All three are cached. Contracts that used to grep the `justfile` for a command
use `gate_issues`; contracts about ordering use `gate_labels`. Do not grow a
local copy of these in a test file -- eight files did during the port, and the
copies drifted.

Two lessons paid for here:

**The double is not the thing.** `Resource.environment` is a method;
`Workspace.environment` was a property. Every funnel test passed because they
used a recorder written to match the protocol, and the one resource every
isolated command actually holds raised `TypeError`. Guards should walk the real
subclasses.

**A guard built from the current state asserts nothing.** The exclusivity guard
passed on first write because I listed what was already non-exclusive. Write the
claim, watch it fail, then make it true.

Break every guard once and watch it go red. Clear `__pycache__` between runs.

## See also

`/dev-just` for the public surface, `/dev-testing` for the suites,
`/release-process` for what the release lanes must guarantee.
