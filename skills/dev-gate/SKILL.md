---
name: dev-gate
description: How capsem-gate works and how to add or change a gate command. Use when touching build, test, or release logic, or when a boundary/primitive/contention guard fails.
---

# The build and release gate

The justfile dispatches; `src/capsem/gate/` decides. No recipe carries a shell
body, none exceeds five lines, and both are contract tests rather than
conventions.

This exists because the justfile reached 2457 lines, roughly 2070 of them
inline `bash`. None of it could be unit tested, so every defect was found by
running the forty-minute gate and reading the wreckage: an installer handed a
manifest URL before anything wrote the manifest, a version built from
`$(date +%s)`, a log stream opened under a name rotation had already moved, an
asset compatibility floor hardcoded above the binary shipping beside it.

## The five layers

```
primitives     errors  host  config  configschema  proc  docker
harness        actions  fileactions  context      what can be done
               execution  plan                    ordering, derived
               lifecycle  locks                   teardown, exclusion
               runlog  runhistory  timing  disk    observability, bounds
               command  cli                        one shape per command
capabilities   workspace  service  profiles  toolchain  pytestsuite  audits
               imagebuild  initrd  hostimage  hostpackage  vmproofs  smoke
commands       testmodules  vmmodules  release  assets  install  ...
```

A layer composes the one below and never imports the one above.

## Adding a command

```python
class MyCommand(GateCommand, name="my-command", help="one line for --help"):
    exclusive = True          # needs the machine to itself

    def resources(self):      # acquired in order, released in reverse
        return (Workspace(self._config),)

    def plan(self) -> Plan:
        plan = Plan(self.name)
        first = plan.add(step("build", Run(["cargo", "build"])))
        plan.add(step("verify", Run(["cargo", "test"])), after=(first,))
        return plan
```

Then add the module to `COMMAND_MODULES` in `cli.py` so its subclass registers.

**`execute()` is never overridden.** A contract test fails if a subclass defines
it, because a command that bypasses it bypasses teardown, the machine lock and
the run log at once.

## The rules, and why each exists

**Order is declared, never written.** `after=(...)` builds a graph;
`graphlib.TopologicalSorter` decides the sequence. A cycle is a plan-time error
naming the steps. Whatever the sort makes simultaneously ready is independent by
construction, so parallelism is derived rather than chosen. Never sequence by
putting one `plan.add` above another.

**Contention is declared in config.** Two steps can be genuinely independent
and still unable to share the machine. Name it in `[execution.exclusives]` with
the reason, and claim it with `contends=(config.exclusive("apple_vz"),)`. The
existing four came verbatim from shell comments: the Apple VZ launch budget,
the single service-scoped snapshot lock, the binaries `cargo build` replaces
under a running VM test, the Docker disk budget.

**Work goes through primitives.** `Run`, `Script`, `Shell`, `Launch`, `Copy`,
`Remove`, `MakeDir`, `Symlink`, `AtomicReplace`, `Hash`, `RequireFile`,
`RequireNonEmpty`. Never `shutil` or `subprocess` directly — a guard enforces
it, because work that goes around them is invisible to the dry run and the run
log. `Call` exists as a bridge for work not yet expressed as primitives, and
renders as prose rather than argv precisely so it is unpleasant to leave.

**Every value comes from `config/gate.toml`.** No path, filename, architecture
name or channel name in code. `tests/test_gate_has_no_literal_data.py` catches
them; it has already caught a module carrying its own copy of a list config
declared, and a `tests/` root two call sites were free to disagree about.

**Teardown is a stack.** `held(a, b, c)` acquires in order, releases in
reverse, and runs `preserve` before release because release destroys the
evidence. Never write a `finally` that removes a directory — that is a
`Resource` that has not been written yet.

## Asking without running

```bash
uv run capsem-gate <command> --dry-run    # every step, every action, real argv
uv run capsem-gate <command> --graph      # the same thing as a diagram
uv run capsem-gate <command> --timing     # where the time went, on the way out
```

All three exist on every command by construction, declared once on the shared
parser. A dry run must never touch the machine: `render()` is separate from
`perform()` for exactly this reason.

## After a failure

```bash
uv run capsem-gate runs last --failed
```

Every run writes `target/gate-runs/<id>/` with a validated JSONL event stream, a
log per step, and a summary. The timing report leads with the **critical path** —
the longest chain, not the slowest step, because shortening a step that runs
beside something longer changes nothing.

```bash
uv run capsem-gate gc --dry-run    # what disk the gate is holding, per tree
```

## The guards that will fail you

| Test | What it holds |
|---|---|
| `test_gate_boundary.py` | no recipe has a shell body; ≤5 lines; modules ≤300 lines; `ty` strict on `src/` |
| `test_gate_primitives_are_the_only_way.py` | only the harness touches the machine; only `plan` schedules; nothing kills by name |
| `test_gate_has_no_literal_data.py` | no path, architecture or channel spelled in code |
| `test_gate_command.py` | `execute` never overridden; every command has `--dry-run` |
| `test_gate_plan.py` | order derived, cycles caught, contention honoured, failures aggregated |
| `test_gate_lifecycle.py` | acquire order, reverse release, preserve before release |

## Testing a command

Use `RecordingRunner` from `tests/helpers/gate.py`; it records commands instead
of running them, so ordering is assertable without Docker, a VM or a network.
Assert **edges**, not positions — "clippy runs after the frontend build" holds
however the source is arranged.

Every guard must be observed failing. Break the thing it guards, watch that test
alone go red, revert. Clear `__pycache__` between runs: a stale one has made a
reverted mutation look still-red in this codebase. Mutation testing caught four
tests here that were passing for the wrong reason.

## See also

`/dev-just` for the public command surface, `/dev-testing` for the test suites,
`/release-process` for what the release lanes must guarantee.
