# Application Delivery and GUI Programme

Status: **Spike 0 active on `v1.6`; tranche implementation not started**
Authority: this file defines programme order; each tranche file owns its own
implementation scope and acceptance gates
Shared test specification: [`whiteharbor-test-plan.md`](./whiteharbor-test-plan.md)
OAuth/browser specification: [`browser_spike.md`](./browser_spike.md)

## Outcome

Capsem profiles become the sole declarative catalogue for installable and
launchable applications. `capsem-admin` exclusively authors, validates,
materializes, builds, and releases that catalogue. The product first proves the
new installation mechanics while retiring `co-work` and changing the existing
`code` profile's product name to **Terminal**. Only after those foundations are
independently green does Capsem add a
Debian GUI profile using single-application Xpra forwarding over virtio-vsock
and Capsem-owned browser/OAuth brokerage.

This is deliberately one feasibility spike followed by three independently
shippable tranches. The programme is developed on the `v1.6` branch so useful
spike work can be reviewed, committed, and carried into the owning tranche:

```text
Spike 0: end-to-end Claude Web UI on macOS/Apple VZ (active on v1.6)
    |
    | records evidence and retains useful reviewed code/tests
    v
Tranche 1: robust application automation and delivery
    |
    v
Tranche 2: profile-owned app recommendation/execution, proven on Terminal
    |
    v
Tranche 3: GUI transport and authenticated desktop agents
```

The feasibility spike and three tranche plans are:

0. [`spike-0-claude-feasibility.md`](./spike-0-claude-feasibility.md)
1. [`tranche-1-app-delivery.md`](./tranche-1-app-delivery.md)
2. [`tranche-2-terminal-apps.md`](./tranche-2-terminal-apps.md)
3. [`tranche-3-gui.md`](./tranche-3-gui.md)

Spike 0 investigates the GUI outcome and may run before Tranches 1 and 2. The
realized Tranche 3 implementation may start only after those foundations pass.

## Programme invariants

These rules apply to all three tranches.

1. **Debian remains the guest distribution.** Prefer Debian packages and a
   small dependency closure. Do not assemble a private desktop/window-manager
   stack.
2. **The profile manifest is the only application catalogue.** No probe
   catalogue, named-app test table, frontend app list, or installer registry
   duplicates it.
3. **`capsem-admin` owns profile authoring.** Source mutation, validation,
   architecture checks, artifact pin materialization, image construction, and
   release eligibility use one admin-owned Rust contract. Runtime services and
   frontends receive a read-only materialized projection.
4. **Application desired state is Ansible.** Each application owns checked-in,
   policy-limited `install`, `verify_installed`, `test`, and `validate`
   playbooks. Debian `ansible-core` runs locally without SSH, Runner, downloaded
   collections, or runtime roles.
5. **Ansible is not the process supervisor.** Long-lived CLI/PTY and GUI
   applications launch through Capsem's typed execution/app-instance
   lifecycle.
6. **No network-to-shell installers.** `curl | sh`, `wget | bash`, mutable
   `latest`, remote installer scripts, Docker-only install recipes, and copied
   per-app shell logic are contract failures.
7. **Delivery policy is explicit.** Every app is `builtin`,
   `on_demand_cached`, or `on_demand_nocache`. Cache bytes are never installed
   truth. Until the shared cache exists, cached delivery fails with a typed
   capability error rather than silently changing semantics.
8. **Observed state is independently verified.** Receipts record provenance;
   only the read-only verification playbook can establish `present`.
9. **Execution has one implementation.** `capsem run`, session execution,
   recommended apps, automation, app tests, and launches share resolution,
   security-rail evaluation, process IPC, timeout, cancellation, bounded I/O,
   audit, logging, trace propagation, and result mapping.
10. **OpenTelemetry is designed in.** The common Rust telemetry bootstrap emits
    structured local logs and optional OTLP traces/metrics. Guest automation
    emits bounded typed callback events; no guest collector/exporter is added.
11. **Security rails make security decisions.** UI prompts communicate a
    pending browser/navigation action; they do not invent a second approval
    policy.
12. **The guest has no browser.** GUI OAuth uses the Capsem browser shim,
    Capsem-owned callback, credential broker, and an explicit provider/app
    adapter. The VM never performs a cross-VM credential lookup and never
    receives the broker-private OAuth URL or credentials.
13. **White Harbor is mechanical.** It discovers materialized apps, invokes
    their declared operations, and requires the application-owned validation
    proof. Exit code zero alone is not functional proof.
14. **Release CI remains authority.** Each tranche adds focused feedback, but
    completion ultimately passes the repository's exact `just test` and
    exact-SHA release qualification rules.
15. **Existing Docker image definitions are the build baseline.** The GUI
    profile extends `config/docker/Dockerfile.rootfs.j2` and the existing
    `config/docker/image/` build/manifest/security/environment inputs through
    Capsem Admin. It does not create a parallel container/image build stack.

## Tranche boundaries

### Tranche 1 — application automation and delivery

Owns the admin authoring/materialization contract, four-playbook automation,
artifact delivery modes, independent observed state, provenance, typed
automation events, and installation telemetry. It is useful and releasable
without a recommended application or GUI.

It also retires `co-work` from new profile selection and changes the stable
`code` profile's user-facing name to **Terminal** while keeping `id = "code"`
for stored/session identity. This naming/catalogue change lands with the
Ansible migration so the product presents the profile model Capsem is actually
shipping; it does not wait for application recommendation work.

Exit means a permanent deterministic terminal fixture can be authored through
`capsem-admin`, materialized, installed in a real VM, independently verified,
tested, validated, rerun with zero convergence changes, drifted, repaired,
audited, and diagnosed without Docker/install one-liners.

### Tranche 2 — Terminal profile recommendation and execution

Owns the profile application catalogue exposed to runtime clients, one
recommended app id, the shared execution API, PTY/capture modes, app lifecycle,
and migration of the already-proven Terminal (`id = "code"`) tools. It consumes Tranche 1 and
does not add Xpra, GUI routes, OAuth, or browser shims.

Exit means Terminal recommends `codex-cli` by app id; Codex CLI, Claude Code,
AGY, and every other approved Terminal-profile application install and execute from
their own profile entries; `capsem run` with no command uses the recommendation;
the explicit argv/shell escape hatches still work; and every path produces the
same security, audit, telemetry, cancellation, and result semantics.

### Tranche 3 — GUI and authenticated desktop agents

Owns the GUI profile/surface router, Xpra direct-vsock transport, gateway HTML5
relay, deterministic GUI fixture, browser shim/callback/broker integration,
Claude Desktop first proof, Antigravity second proof, and the mandatory
kernel-vsock versus rust-vmm bake-off.

Exit means a real authenticated desktop agent completes a run-scoped task and
manifest-owned validation through the Capsem UI with no VNC, window manager,
guest TCP listener, guest browser, socat, websockify, or credential exposure.

### Spike 0 — disposable GUI feasibility

The complete isolated experiment, evidence requirements, and binary go/no-go
gates are owned by
[`spike-0-claude-feasibility.md`](./spike-0-claude-feasibility.md). Spike files,
images, credentials, and vendor artifacts do not enter git or a release image.
Temporary install or relay logic is deleted, never promoted, and cannot waive
any production tranche gate. Its mandatory decision is whether the user finds
Claude responsive through the complete Capsem Web/Xpra/Apple-VZ path on the
current Mac. Authentication is an optional second stage with a separate result.
The spike deliberately does not build Linux/KVM or x86_64.

## Cross-tranche ownership

| Concern | Tranche that creates it | Later consumers |
| --- | --- | --- |
| Admin-owned app schema/materialization | 1 | 2, 3 |
| Four-playbook automation and observed state | 1 | 2, 3 |
| Delivery modes and future blob-cache contract | 1 | 2, 3 |
| Shared typed execution service | 2 | 3 |
| Profile recommendation and app lifecycle | 2 | 3 |
| Terminal fixture and terminal White Harbor lane | 1/2 | 3 |
| Xpra/vsock GUI transport | 3 | GUI apps only |
| Browser ceremony and Capsem callback | 3 | OAuth-capable apps |
| OTel and logger correlation | 1, extended in 2/3 | all operations |
| White Harbor production catalogue sweep | grows per tranche | release gate |

No later tranche may create a compatibility implementation for an earlier
contract. If the GUI work discovers an installation or execution deficiency,
the fix belongs in the owning earlier subsystem and its tests.

## Sprinty handoff

These plans use separate Sprinty sprints. Do not create one umbrella
implementation sprint that hides their individual exit gates. Spike 0 uses the
**Claude Web GUI feasibility** investigation sprint on `v1.6`; it is never the
Tranche 3 implementation sprint.

At the beginning of each implementation session:

1. call `mcp__sprinty.info()`;
2. create or continue exactly the sprint named by that tranche plan;
3. copy the plan's scope, exclusions, exact file map, and exit gates into the
   sprint context;
4. create a dedicated Sprinty item before each file-owning or risk-owning body
   of work;
5. record focused verification and final `just test`/release-gate status before
   closing the sprint;
6. do not start the next tranche until the prior tranche's exit gate is green
   or an explicit architectural decision records why the dependency changed.

The design documents are durable planning inputs. Sprinty becomes the source of
truth only when implementation of that tranche actually starts.

## Programme completion

The programme is complete only when all three tranche gates pass, White Harbor
mechanically exercises every advertised app on every advertised architecture,
Winterfell proves lifecycle and broker reuse behavior, Ironbank proves the full
security/ledger/OTel truth without secret leakage, the mandatory vsock bake-off
has selected a backend using archived evidence, and exact-SHA release
qualification passes.
