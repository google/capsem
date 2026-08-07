# Spike 0 — End-to-End Claude Web UI on macOS

Status: **active investigation on `v0.7`**
Sprinty sprint: **Claude Web GUI feasibility**
Host: **macOS on Apple silicon**
Hypervisor: **Apple Virtualization.framework**
Guest: **arm64 Debian**
Client: **ordinary Web browser through the Capsem UI**
Working branch: **`v0.7`; useful source, tests, instrumentation, planning, and
sanitized evidence may be committed**
Result artifact: `tmp/gui/spike-0-result.md`
Real implementation, if approved:
[`tranche-3-gui.md`](./tranche-3-gui.md)

## Outcome

The mandatory outcome is a live URL in the Capsem Web UI where the user can
personally interact with Claude Desktop running inside an arm64 Debian VM and
decide whether the experience is responsive enough to justify the realized GUI
work.

The mandatory end-to-end path is:

```text
user's Web browser
  -> Capsem Web UI
  -> authenticated, instance-scoped WebSocket
  -> private host Unix socket
  -> temporary Capsem-owned host relay
  -> Apple VZ host-to-guest virtio-vsock
  -> Xpra/Xdummy in arm64 Debian
  -> Claude Desktop window
```

A screenshot, Xpra debug page, direct TCP connection, native Xpra client, or
host-launched Claude is not the outcome. The user must drive the actual browser
surface through the complete proposed transport.

Authentication is a follow-on experiment only. It starts after the Web UI is
visible, measurable, and accepted for responsiveness. Authentication failure
does not change a successful UI feasibility result; it produces a separate
auth result and blocks only the OAuth/agent portion of realized Tranche 3.

## Decisions

The result document records two independent decisions.

### Mandatory UI decision

- **UI_GO** — the end-to-end Web surface passes every UI hard gate, objective
  measurements are inside the predeclared budgets, and the user explicitly
  accepts responsiveness.
- **UI_NO_GO** — the surface is visible but unusable, too latent, unstable, or
  requires a forbidden architectural component.
- **UI_NOT_PROVEN** — the Web surface or a required measurement could not be
  made available. This is not success.

`UI_GO` authorizes planning confidence for Tranche 3's transport/UI legs after
Tranches 1 and 2. It does not authorize shipping.

### Optional authentication decision

- **AUTH_GO** — the real-browser, Capsem-callback, broker, Claude-adapter, and
  independently validated agent path works.
- **AUTH_NO_GO** — a provider or architecture limitation makes that path
  nonviable.
- **AUTH_NOT_PROVEN** — prerequisites were unavailable or the attempt was
  inconclusive.
- **AUTH_NOT_RUN** — the UI spike ended after its mandatory objective.

`AUTH_NOT_RUN`, `AUTH_NOT_PROVEN`, or `AUTH_NO_GO` does not downgrade
`UI_GO`. It means the OAuth/agent implementation leg requires its own follow-up
spike or remains blocked.

## Deliberately narrow platform scope

Spike 0 runs only on the current macOS/Apple-silicon development machine:

| Dimension | Spike row |
| --- | --- |
| Host | macOS arm64 |
| Hypervisor | Apple Virtualization.framework |
| Guest | arm64 Debian |
| Profile | `gui`, authored and built through `capsem-admin` |
| Application | official arm64 Claude Desktop Linux package |
| Client | Web browser through Capsem Web UI |
| Transport | Xpra HTML5 over Apple VZ virtio-vsock relay |

The spike does not build on Linux, run KVM, test `/dev/vhost-vsock`, implement
rust-vmm, or prove x86_64. Those are realized Tranche 3 and White Harbor
obligations. Their absence cannot block `UI_GO`, and `UI_GO` must not be cited as
evidence that they work.

Tauri is also outside the mandatory spike. The Web path is the harder universal
surface and the one the user must evaluate.

## Branch and artifact policy

The spike authors the real `config/profiles/gui` source contract and builds it
through `capsem-admin`. The profile may pin Claude Desktop and Xpra in its
profile-owned package/build inputs before the future Ansible/on-demand app
delivery model exists, but it does not bypass profile ownership, admin
validation, materialization, image construction, manifests, build ledgers, or
OBOM evidence.

Work directly on the `v0.7` branch. Useful source, tests, instrumentation,
planning, and sanitized evidence should be committed at functional milestones
so the work can continue through the later tranches. Spike-only vendor assets,
generated images, raw captures, credentials, tokens, and machine-sensitive
evidence live under ignored `tmp/gui/spike-0/` and must not enter git. Do not use
a sidecar gateway or a standalone Xpra demo page. Every edit must be minimal,
inspectable, and annotated with the future product owner it represents.

The browser-visible lane uses the running Capsem gateway's existing loopback
listener, token authentication middleware, CORS/origin rules, tracing layer,
service UDS, and companion-process lifecycle. It is bound to one verified
VM/app instance and cannot become a generic TCP/WebSocket proxy.

The spike may remain live while the user tests it. It must expose the URL,
report readiness, continue collecting bounded metrics, and wait for explicit
user sign-off or a predeclared idle deadline before teardown.

## Forbidden shortcuts

- no secret, credential, token, vendor binary, generated image, raw capture,
  machine-sensitive path, or unsanitized evidence is staged or committed;
- no Spike 0 code is treated as production-ready without focused review and
  tests; useful implementation is retained on `v0.7` under its owning Sprinty
  items and hardened through the later tranche gates;
- no `curl | sh`, `wget | bash`, mutable `latest`, or remote installer script;
- no VNC/noVNC, Spice desktop, desktop environment, window manager, guest
  browser, guest GUI TCP listener, socat, websockify, or generic proxy;
- no direct Xpra debug URL or native Xpra client for the user acceptance gate;
- no bypass of the existing Capsem Web authentication/session boundary;
- no claim that Ansible delivery, Capsem Admin authoring, Linux, x86_64,
  rust-vmm, White Harbor, or release qualification has been proven;
- no login screenshot, process exit zero, or application-reported success used
  as a responsiveness or functional oracle.

## Preparation

Before building, record in the sanitized result:

- exact Capsem candidate SHA, active branch, and existing working-tree state;
- macOS version, Apple silicon model, host CPU/memory, and browser/version;
- Debian version, guest kernel, vCPU/RAM, and display size;
- exact Claude package version/origin/architecture and a sanitized digest;
- exact Xpra server and HTML5 client versions;
- Debian dependency closure and image-size delta;
- opaque spike run, VM, app-instance, and trace ids;
- local network conditions and the optional shaped-network scenarios;
- readiness, latency, resource, and idle-timeout budgets below.

The official Claude artifact is acquired through a policy-mediated path,
version-pinned, and digest-verified. Prefer Debian packages for Xpra,
Xorg/Xdummy, D-Bus, XDG integration, fonts, and graphics dependencies. Record
every non-Debian runtime component and why it was required.

## Mandatory gateway and UI integration

The spike must use the existing terminal path as an architectural precedent,
not bypass it. Today `capsem-gateway` exposes `/terminal/{id}`, obtains its
token through `/token`, connects to the per-VM mode-`0600` UDS, and runs behind
the gateway auth/CORS/tracing layers. Spike 0 adds the equivalent GUI lane on
`v0.7`:

```text
GET /gui/{vm_id}/{app_instance_id}?token=<gateway token>
GET /vms/{vm_id}/app-instances/{app_instance_id}/gui-metrics
```

The WebSocket is implemented by `capsem-gateway`. The metrics request is an
explicit gateway proxy route to `capsem-service` over the existing service UDS;
the service returns the latest bounded one-second summary owned by the live
instance. Neither route addresses a sidecar.

The actual spike hop ownership is:

```text
frontend/src/pages/vm/gui-spike.astro
  -> GuiSpikeFrame.svelte fetches token from the existing gateway /token
  -> browser WebSocket to capsem-gateway /gui/{vm}/{instance}
  -> gateway auth middleware validates the normal gateway token and Origin
  -> GUI route validates both identifiers
  -> GUI route asks capsem-service whether the live instance belongs to VM
  -> gateway connects only the service/process-selected private GUI UDS
  -> capsem-process owns the bounded UDS <-> Apple-VZ-vsock relay
  -> guest Xpra AF_VSOCK endpoint owns only that app/display
```

The gateway may not accept a UDS path, guest port, upstream URL, Xpra command,
or process id from the browser. The browser cannot derive the UDS. The gateway
must fail closed when the service does not confirm the VM/instance pair, when
the process-owned socket is absent, or when the instance is no longer live.

The query-token allowance in `auth.rs` is extended narrowly from the existing
terminal/events WebSocket cases to `/gui/`; it is not generalized to arbitrary
paths. The token, raw query string, and WebSocket URL are excluded from logs and
the result artifact.

The existing terminal relay's 64 KiB/16 ms text-oriented batching must not be
copied blindly into the GUI hot path. The GUI relay byte-forwards Xpra frames
with explicit bounded queues, backpressure, cancellation, and per-direction
metrics. Any batching/coalescing decision is recorded because it directly
affects input latency and FPS.

The Capsem UI embeds a sandboxed same-origin GUI iframe analogous to
`frontend/src/pages/vm/terminal.astro`. Its CSP allows connections only to the
normal loopback Capsem gateway. The iframe owns the matched Xpra HTML5 client
and WebSocket lifecycle. The parent Capsem shell owns lifecycle state, manual
test controls, and the live performance panel. There is no link or fallback to
an Xpra debug page.

### Initial Spike 0 change surface

The initial implementation is limited to these owned changes on `v0.7`:

- `config/profiles/gui/` — the sole GUI profile source contract, including
  package inputs, build hook, root seed, security inputs, and profile metadata;
- `crates/capsem-admin/` and its tests only where the existing public
  `profile validate|check|materialize` or `image build` rails cannot yet express
  the GUI profile; no GUI-specific authoring command or backend-owned catalog;
- `crates/capsem-gateway/src/main.rs` — mount the exact
  `/gui/{vm_id}/{app_instance_id}` WebSocket and explicit GUI-metrics proxy
  routes inside the existing auth/CORS/trace stack;
- `crates/capsem-gateway/src/auth.rs` — narrowly allow WebSocket query-token
  auth for `/gui/`;
- temporary `crates/capsem-gateway/src/gui_spike.rs` — ownership lookup,
  process-UDS connection, bounded binary relay, counters, and teardown;
- `crates/capsem-service/src/main.rs` and `api.rs` — temporary typed
  create/status/stop app-instance and GUI-metrics projection; no generic relay
  inputs;
- `crates/capsem-process/src/main.rs`, `vsock.rs`, and temporary
  `gui_spike.rs` — Apple-VZ-vsock connection ownership, private UDS, Xpra relay,
  resource/queue counters, cancellation, and cleanup;
- `crates/capsem-proto/src/lib.rs`/`ipc.rs` — fixed GUI service port and typed
  spike lifecycle/metrics messages if required;
- `frontend/src/pages/vm/gui-spike.astro` and temporary
  `frontend/src/lib/components/gui/GuiSpikeFrame.svelte` — sandboxed Xpra client,
  gateway token/WebSocket lifecycle, frame/input instrumentation;
- the existing shell/session component needed to show the GUI iframe and live
  performance panel inside Capsem rather than as a standalone page;
- version-matched Xpra HTML5 client assets in the ignored spike workspace, not
  the repository's committed static tree.

The result maps each successful component to its realized Tranche 3 owner.
Useful source, tests, and OTel instrumentation are committed on `v0.7` at
functional milestones. If an incomplete experimental tail remains, save an
optional sanitized source-only diff as `tmp/gui/spike-0-review.patch`. Exclude
vendor assets, generated files, tokens, URLs, credentials, machine paths, and
raw evidence. Neither a spike commit nor the optional patch is treated as
production-quality until the owning tranche review and gates pass.

## Stage A — mandatory end-to-end Web UI

### A1. Author and build the GUI profile

- create `config/profiles/gui` as the only GUI profile source contract;
- run `capsem-admin profile validate`, `profile check`, and `profile
  materialize`, then build through `capsem-admin image build` and the existing
  `just build-assets gui arm64` rail;
- start from the normal arm64 Debian guest base through the existing
  `config/docker/Dockerfile.rootfs.j2` and `config/docker/image/` build,
  manifest, security, and VM-environment inputs;
- do not reuse the `code` profile, author generated backend workspaces, invoke
  the Python backend directly, or create an unrelated GUI Dockerfile/builder;
- install the exact verified Claude package and dependency closure;
- install only the minimal Xpra/Xdummy/D-Bus/XDG/font/graphics runtime;
- install the temporary fixed launcher/relay endpoint required for the spike;
- prove the image contains no guest browser, WM, desktop environment, VNC/Spice
  server, socat, websockify, or added SSH server;
- record package closure and compressed/uncompressed image delta.

### A2. Launch one Claude window without a WM

- boot the image under Apple VZ;
- launch Claude under Xpra/Xdummy with exit-with-child semantics;
- bind Xpra to the declared guest AF_VSOCK port;
- attribute Claude, its descendants, Xpra display, and guest socket to one
  host-issued spike app-instance id;
- prove one nonempty Claude window exists and remains alive without an
  interactive guest shell supervising it.

### A3. Complete the proposed host transport

- connect host-to-guest with Apple VZ's virtio-socket API;
- relay bounded Xpra bytes to a private mode-`0600` host Unix socket;
- register the instance with `capsem-service` and expose only the running
  `capsem-gateway` route
  `/gui/{vm_id}/{app_instance_id}?token=<normal gateway token>`;
- require the gateway to confirm VM/instance ownership through the service
  before connecting the process-owned UDS;
- use the exact version-matched Xpra HTML5 client;
- prove there is no guest GUI TCP listener and no caller-selected host/guest
  port, URL, socket, process, or Xpra option.

A temporary loopback lane may diagnose Xpra/HTML5 in isolation, but it cannot
satisfy this stage or be the URL given to the user.

### A4. Expose the live Capsem Web URL

The spike runner must not declare success merely because a frame was decoded.
It must:

1. start the real `capsem-service`, `capsem-process`, `capsem-gateway`, and
   existing Capsem Web UI from the `v0.7` branch;
2. register the disposable VM/app instance with `capsem-service`;
3. have the Web iframe fetch the normal token from `capsem-gateway /token` and
   connect to the exact `/gui/{vm}/{instance}` gateway WebSocket;
4. make the gateway verify ownership and connect the process-owned UDS;
5. render the Xpra HTML5 canvas inside the Capsem UI shell;
6. wait for a first-frame readiness signal;
7. provide the user one ordinary `http://localhost...` or configured Capsem Web
   URL to open in their normal browser;
8. keep the session alive for manual evaluation;
9. display the live performance panel defined below without exposing secrets or
   generic debug controls.

No direct Xpra address is part of the acceptance instructions.

### A5. User responsiveness session

The user personally performs this checklist in the browser:

- observe the first Claude frame and normal visual rendering;
- click buttons/links and open/close an application dialog;
- focus a text field and type a sustained paragraph, including rapid typing,
  deletion, selection, shortcuts, and Unicode;
- scroll a long surface and judge continuity/tearing;
- move the pointer rapidly and perform repeated clicks;
- resize the Capsem panel/window through small and large dimensions;
- switch browser tab away and back;
- disconnect/reconnect once without restarting Claude;
- leave the session idle, then resume interaction;
- state an explicit verdict: `responsive`, `marginal`, or `unusable`, with brief
  notes identifying visible latency, tearing, blur, dropped input, resize lag,
  or instability.

The runner must not tear down before this verdict is captured unless the user
requests teardown or the predeclared idle deadline expires.

`UI_GO` requires the user verdict `responsive`. `marginal` is `UI_NO_GO` unless
the user explicitly requests a second run with a named configuration change;
the original result remains recorded.

### A6. Objective measurements

Measure the same live Web path during the manual session. Samples use monotonic
timestamps and one-second resource windows; store p50, p95, p99, maximum, and
time-series artifact digests where applicable.

#### Latency

- launch request to Xpra process;
- process start to first window;
- window to first decoded browser frame;
- browser keyboard/pointer event to gateway receipt;
- gateway receipt to process relay write;
- process relay to guest/Xpra receipt where the protocol exposes it;
- browser input event to the next correlated Xpra damage/update decoded and
  presented by the HTML5 canvas (the spike's best real-Claude
  input-to-visible-update proxy);
- resize request to stable resized frame;
- reconnect request to resumed frame;

Also record WebSocket ping RTT separately so transport RTT is not confused with
application/display acknowledgement. Real Claude cannot provide a semantic
input nonce; the deterministic Test GUI App in Tranche 3 supplies that exact
measurement later.

#### FPS and frame pacing

Xpra forwards damage regions rather than a fixed video stream, so report both
protocol updates and browser presentations instead of inventing one FPS number:

- Xpra damage/update packets per second received by the browser;
- decoded canvas presentation/update batches per second;
- requested vs completed `requestAnimationFrame` callbacks;
- inter-presentation interval p50/p95/p99;
- long-frame/jank counts over 50 ms and 100 ms;
- dropped, superseded, coalesced, or late updates;
- measurements for idle, sustained typing, rapid scrolling, dialog animation,
  and continuous resize workloads.

The live panel labels these **Xpra updates/s** and **presented FPS**. It never
labels damage-packet count as display refresh FPS.

#### CPU

Collect average, p95, and maximum CPU for each workload and idle:

- guest Claude process tree;
- guest Xpra/Xorg/Xdummy;
- host per-VM `capsem-process`;
- host `capsem-gateway`;
- browser renderer process for the Capsem GUI tab where macOS/browser tooling
  can attribute it;
- total host CPU and VM/vCPU utilization.

Report percentages with their denominator (`one core`, `allocated guest vCPU`,
or `whole host`) so a value such as 100% is not ambiguous.

#### Memory

Image size is not memory. Record both separately. Runtime memory measurements
include:

- guest Claude RSS and, where available, PSS from `/proc/*/smaps_rollup`;
- guest Xpra/Xorg/Xdummy RSS/PSS;
- total guest used/available memory and swap activity;
- Apple VZ VM process/helper RSS on the host;
- host `capsem-process` and `capsem-gateway` RSS;
- browser renderer RSS for the Capsem GUI tab where attributable;
- current/peak GUI relay queue bytes in both directions;
- baseline before launch, post-first-frame, peak interaction, five-minute idle,
  reconnect, and post-stop values.

The result explicitly separates Claude/Electron cost, Xpra/display cost, VM
overhead, Capsem relay/gateway overhead, and browser-client overhead. A single
host RSS number is insufficient.

Use the GUI profile's ordinary spike configuration once and report the real
CPU and memory it consumes. This spike is not a VM-sizing matrix: do not rerun
synthetic vCPU/RAM envelopes merely to manufacture a minimum. The result should
make the observed Claude, Xpra, VM, relay, gateway, and browser costs easy to
compare so later profile sizing can use evidence.

#### Bandwidth, codec, and queues

- WebSocket and vsock bytes/sec in each direction;
- Xpra encoded bytes versus estimated damaged raw pixels when available;
- codec/encoding, quality, speed, subsampling, and any automatic transitions;
- current/maximum queue bytes and messages at process, gateway, and browser;
- backpressure duration and dropped/coalesced update counts;
- teardown duration and remaining process/socket count.

#### Live performance panel

While the user interacts, the parent Capsem UI must visibly show, updated about
once per second:

```text
presented FPS | Xpra updates/s | input-to-update p95 | gateway RTT
down/up Mbps | codec/quality | dropped/coalesced updates | max queue
guest CPU/RSS (Claude + Xpra) | host CPU/RSS (process + gateway) | VM memory
```

The panel reads typed metrics projected through `capsem-service` and the normal
gateway API/event path. It does not scrape logs or connect directly to the VM,
process UDS, or Xpra server. Raw high-frequency samples stay bounded; the UI
gets coalesced one-second summaries.

Record distributions and raw sample artifact digests, not only averages.

### A7. Local and shaped-network checks

The mandatory gate is local macOS browser responsiveness. After it passes,
optionally repeat the same browser checklist with host-side shaping that models:

- regional: approximately 40 ms RTT and 20 Mbps;
- distant: approximately 120 ms RTT and 10 Mbps;
- a small loss/jitter case suitable for observing Xpra adaptation.

These optional rows establish codec/compression direction. They do not downgrade
a local `UI_GO`, but their results must not be represented as supported remote
SLOs. The realized deterministic GUI benchmark owns final remote budgets.

### A8. Teardown and failure behavior

After user sign-off:

- stop the app instance and prove Claude descendants and Xpra are reaped;
- close vsock, private UDS, authenticated WebSocket, and temporary relay;
- repeat one browser disconnect and one forced Xpra failure;
- prove neither produces a false ready/running result;
- verify no listener, process, overlay mount, or temporary credential remains.

## Stage A hard gates

Every gate is mandatory for `UI_GO`.

| Gate | Pass condition |
| --- | --- |
| UI-0 Provenance | Official pinned arm64 Claude package is digest-verified; no remote installer runs. |
| UI-1 Admin-authored GUI profile | `config/profiles/gui` validates, checks, materializes, and builds through `capsem-admin`; its arm64 image boots under Apple VZ, its dependency/image delta and OBOM/build-ledger evidence are recorded, and forbidden desktop/browser components are absent. |
| UI-2 Single application | Claude owns a usable Xpra window without a WM or supervising guest shell. |
| UI-3 Direct vsock | Xpra traffic crosses Apple VZ virtio-vsock and a private Capsem relay; no guest GUI TCP or generic proxy exists. |
| UI-4 Gateway path | Browser uses the running `capsem-gateway /gui/{vm}/{instance}` route, normal token/auth/origin middleware, service ownership check, and process UDS; no sidecar or direct Xpra URL exists. |
| UI-5 Capsem Web surface | User receives and opens the normal Capsem UI containing the sandboxed GUI iframe, not a debug/native client. |
| UI-6 Interaction | Keyboard, pointer, scrolling, resize, tab suspend/resume, and reconnect all work end to end. |
| UI-7 Responsiveness | User explicitly records `responsive`; latency and frame-pacing measurements meet the budgets below. |
| UI-8 CPU/memory | Real-world per-component CPU and RSS/PSS plus VM/queue memory are measured through launch, interaction, idle, reconnect, and stop. |
| UI-9 FPS/bandwidth | Presented FPS, Xpra update rate, jank, drops, codec, per-direction bandwidth, and queues are measured and visible live. |
| UI-10 Stability | No lost/stuck input, unrecoverable blank frame, unbounded queue/memory growth, OOM, or continuous idle CPU saturation. |
| UI-11 Observability | One run id correlates guest, vsock, process, gateway, browser, input/frame, CPU/memory, failure, and teardown evidence. |
| UI-12 Cleanup | All spike processes, listeners, sockets, temporary images/overlays, captures, raw logs, credentials, and ignored scratch artifacts are removed after sign-off; useful committed `v0.7` work remains. |

## Stage A feasibility budgets

Unless a stricter hardware-specific budget is recorded before the first run:

- process start to first decoded Web frame: **15 seconds maximum**;
- established-session keyboard/pointer to visible acknowledgement:
  **150 ms p95 local**;
- resize request to stable resized frame: **500 ms p95 local**;
- reconnect request to resumed frame: **3 seconds maximum**;
- sustained-scroll presented rate: **24 FPS median or better**, with fewer than
  **5%** of presentation intervals over 100 ms;
- sustained resize: **20 presented FPS median or better** while active;
- zero dropped/stuck input events during the manual checklist;
- GUI relay queues remain bounded and report any time spent at their cap;
- no OOM, swap storm, unbounded memory/queue growth, or continuous idle CPU
  saturation;
- missing CPU or memory measurements produce `UI_NOT_PROVEN`; observed usage is
  reported rather than judged against an invented resource envelope.

These are feasibility thresholds, not final product SLOs.

## Stage B — optional authentication and agent extension

Run only after Stage A has produced a browser-visible UI and the user elects to
continue. Stage B follows the complete security/broker design in
[`browser_spike.md`](./browser_spike.md).

### B1. Browser interception

- register the Capsem shim as Claude's browser/`xdg-open` handler;
- cause Claude to request login naturally;
- send a typed pending-browser request through a private host-only service;
- obtain the security rail's decision;
- surface only a server-issued request id in the Web UI;
- prove the guest does not open/follow the URL or receive callback/credential
  data.

### B2. Real browser and Capsem callback

- the user's click is the browser gesture that invokes `window.open`;
- the exact broker-private authorization URL opens in a normal tab;
- the provider returns only to a Capsem-owned HTTPS callback;
- no guest loopback, VM IP, guest-selected callback, or custom URI is used;
- bind the original exact authorization URL privately to the broker ceremony,
  provider adapter, Claude application adapter, callback state, and run id.

### B3. Claude completion and broker reuse

- exchange/complete callback facts inside the broker/provider boundary;
- invoke an explicit Claude application-completion adapter;
- prove the already-visible Claude UI transitions to authenticated state;
- destroy VM A and create clean VM B;
- prove broker-owned reuse works according to policy without querying VM A or
  exposing URL/token/cookie/profile/credential data to VM B.

There is no cross-VM VM lookup. Reuse is broker-owned state.

### B4. Real agent proof

- create a clean fixture repository and host-issued nonce;
- ask Claude through the forwarded Web UI to write the nonce to one specified
  file;
- independently validate the exact current-run path/content and absence of
  forbidden changes;
- record only hashes, run identity, and boolean validation;
- stop/relaunch and record authenticated/application behavior.

Claude's own message or exit status is not validation.

### Stage B gates

| Gate | Pass condition |
| --- | --- |
| AUTH-0 Rail | Natural Claude browser request becomes a typed rail-controlled pending request; UI does not invent a second security decision. |
| AUTH-1 Callback | Real browser returns only to a Capsem-owned callback. |
| AUTH-2 Adapter | Explicit Claude adapter reaches authenticated state without guest credential/callback exposure. |
| AUTH-3 Reuse | Clean VM B can use broker-owned binding under policy without VM lookup or credential/profile copying. |
| AUTH-4 Agent work | Current run creates the exact nonce file and independent validation passes. |
| AUTH-5 Hygiene | Raw OAuth URLs/query, callbacks, tokens, cookies, credentials, and browser state are absent from guest/public APIs/logs/OTel/result. |
| AUTH-6 Cleanup | Browser, callback, broker spike grant, VM, app, relay, and temporary state are removed or explicitly revoked. |

All pass yields `AUTH_GO`. Any architectural/provider failure yields
`AUTH_NO_GO`. Missing prerequisites or measurements yield `AUTH_NOT_PROVEN`.
Stopping after Stage A yields `AUTH_NOT_RUN`.

## Observability and evidence

Use one opaque spike run/trace identity across image build, VM, Claude/Xpra,
Apple VZ vsock, relay, gateway, WebSocket, browser frame/input, manual session,
optional browser request/callback/broker/adapter, validation, and teardown.

Allowed evidence includes typed outcome, duration, architecture/version, codec,
RTT, bounded byte/frame/input counts, process/instance ids, resource samples,
and content/artifact hashes. Exclude raw OAuth URLs/query strings, callback
parameters, tokens, cookies, credentials, environment values, browser state,
clipboard data, screen/frame contents, fixture contents, and unbounded output.

The temporary implementation should exercise the intended OTel topology while
it measures the path: browser readiness/input summaries correlate with gateway
WebSocket, gateway-to-UDS, process relay, Apple-VZ-vsock, Xpra first-frame, CPU/
memory sampling, failure, and teardown spans. A successful sanitized patch is
retained as review input for the common telemetry implementation; failed or
misleading instrumentation is documented rather than copied.

The live user URL is ephemeral operational information. It may be sent to the
user during the run but is not retained after teardown.

## Result document

Create `tmp/gui/spike-0-result.md` only when executing the spike. It must begin:

```text
UI decision: UI_GO | UI_NO_GO | UI_NOT_PROVEN
User responsiveness verdict: responsive | marginal | unusable | not_tested
Authentication decision: AUTH_GO | AUTH_NO_GO | AUTH_NOT_PROVEN | AUTH_NOT_RUN
Host/guest: macOS arm64 / Apple VZ / arm64 Debian
Web end-to-end path proven: yes | no
Failed UI gates: <gate ids or none>
Failed auth gates: <gate ids, none, or not_run>
Tranche 3 transport/UI legs feasible after Tranches 1 and 2: yes | no
Tranche 3 OAuth/agent leg feasible: yes | no | not_proven
Sanitized spike patch retained for later review: yes | no
Direct promotion without reimplementation/review/tests allowed: no
```

It also contains:

1. exact sanitized version/dependency/image/hardware facts;
2. the ephemeral URL's route shape but not reusable session material;
3. each UI and optional auth gate with pass/fail/not-proven evidence;
4. the user's responsiveness notes;
5. latency/resource/codec/frame/input tables and sample artifact digests;
6. optional shaped-network results clearly separated from local acceptance;
7. every retained component mapped to its tranche owner and marked for focused
   review and hardening before that tranche closes;
8. failures/blockers and the smallest next experiment;
9. teardown and prohibited-data cleanup proof.

## Exit rule

The mandatory spike may close when Stage A has a decision, the user verdict is
recorded, all UI gates have outcomes, evidence is sanitized, and cleanup is
proven. Stage B may run immediately afterward or remain `AUTH_NOT_RUN` for a
later focused session.

`UI_GO` proves only that the admin-authored GUI profile and Claude are usable
end to end in a Web browser on the macOS/Apple-VZ/arm64-Debian spike row.
Reviewed commits remain on `v0.7` for their owning tranches, but no spike result
waives tranche hardening or gates. It proves nothing about Linux, KVM, x86_64,
final multi-architecture delivery, release security, or qualification.
