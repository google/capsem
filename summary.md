# Capsem Plugin, UI Runtime, and Native Tooling Prototype Summary

Last updated: 2026-06-28

This document is the reusable orientation note for splitting this work across
multiple focus sessions. It records what lives where, what has been proven, what
is still prototype-only, and which lanes should be picked up independently.

The most important rule: this work currently spans two separate worktrees. The
prototype worktree is intentionally isolated. Do not port it into mainline until
the contracts, tests, benchmarks, and export story are locked.

## Worktree Map

### Real Capsem checkout

Path:

```text
/Users/elie/.codex/worktrees/2536/capsem
```

Observed git state:

```text
git rev-parse --show-toplevel -> /Users/elie/.codex/worktrees/2536/capsem
git status --short --branch   -> ## HEAD (no branch)
```

Role:

- This is the real Capsem checkout and the place where eventual integration
  must happen.
- It contains the actual Capsem application, including the real frontend,
  Capsem MCP, Capsem settings, hypervisor, security engine, skills, and runtime
  architecture.
- It is not where the current prototype code lives.
- Treat it as read-only unless a session is explicitly about mainline
  integration.

Integration relevance:

- The prototype must eventually map into the real Capsem frontend and MCP host.
- Capsem already has lower-level capture/logging around models, MCP, tool calls,
  and hypervisor boundaries. Do not invent a separate production telemetry lane
  for this prototype unless a specific missing data point is found.
- The real Capsem UI uses Svelte/Preline and must be the target for renderer
  porting.

### Isolated prototype worktree

Path:

```text
/Users/elie/.codex/worktrees/2536/capsem-plugin-prototype
```

Branch:

```text
codex/plugin-event-prototype
```

Role:

- This is the active isolated prototype branch.
- It contains the JS-to-WASM/plugin spike, Rust plugin engine spike, typed UI
  catalogue, local MCP-shaped tool routes, chat UI shell, workspace stream,
  comments, Loro experiments, Gemini/provider wrapper experiments, Plotly and
  Mermaid rendering, and export prototypes.
- It is intentionally not mainline-ready.
- It has many dirty and untracked files. Do not revert unrelated changes.

Top-level shape:

```text
crates/
  capsem-ai/
  capsem-plugin-engine/
  capsem-plugin-server/
  capsem-ui-catalog/
  capsem-ui-runtime/
docs/
private/
python/
schemas/
scripts/
sprints/
templates/
test/
ui-preview/
```

Current dirty/untracked status is expected. This is a long-running prototype.
The important untracked prototype files include:

```text
crates/capsem-ui-catalog/src/contract_matrix.rs
crates/capsem-ui-catalog/src/editable_state.rs
crates/capsem-ui-catalog/src/workspace.rs
crates/capsem-ui-runtime/
scripts/bench-workspace.ts
scripts/e2e-chat.ts
scripts/e2e-export.ts
scripts/export_artifact.py
scripts/generate-native-artifact-types.ts
scripts/inspect-workspace-session.ts
sprints/mainline-ui-runtime-integration/
sprints/workspace-stream-projection/
test/artifact-topology.test.ts
test/loro-state.test.ts
test/native-workspace.test.ts
ui-preview/src/elements/topologyRegistry.ts
ui-preview/src/generated/
```

## High-Level Product Direction

We explored three connected but separable product tracks:

1. A plugin system where author-facing code is close to TypeScript/JavaScript,
   compiled to WASM, run through Rust-controlled intrinsics, and kept isolated
   from Capsem internals.
2. A typed UI generation/runtime system where agents, plugins, and local tools
   emit strongly typed UI/artifact objects rather than raw ad hoc JSON or direct
   DOM mutations.
3. A local MCP/tool surface that lets an agent build, edit, inspect, render, and
   export artifacts such as cards, tables, charts, diagrams, generated images,
   spreadsheets, slides, and slide decks.

The current strategic split:

- Plugin/WASM runtime remains important, but it is not the fastest path to
  shipping the UI artifact system.
- The UI runtime, typed artifact catalogue, chat surface, local MCP-shaped
  tools, and export path can land ahead of the full WASM plugin system if they
  are locked and tested.
- The plugin runtime can later use the same typed UI/tool contracts.

## Key Decisions Captured

### Plugin execution model

- Plugins should be authored with a fluent API shape, not a low-level raw JSON
  dictionary.
- Desired authoring form is conceptually:

```ts
Plugin("name", {
  onFileCreate(file, context) {
    const head = fs.read(file.path + "/HEAD");
    ui.card(...);
    return file;
  }
});
```

- Callback object names should be descriptive. Avoid generic "event" naming
  when the object itself has a real domain name.
- Context is copied into WASM, passed to callbacks, and not returned. WASM is
  the wall; only the target object can return.
- Intrinsics discussed or spiked: filesystem read/write/delete style ABI,
  HTTP/fetch ABI, model-call ABI, UI emit/render ABI.
- Isolation remains required. Long-term direction is not "WASM in the main
  process and hope." The runtime needs an out-of-process boundary or equivalent
  isolation strategy before production.

### Typed UI model

- Do not expose loose maps when the contract should be enums and typed objects.
- Rust is the contract authority for server/control-plane types.
- TypeScript/Python bindings are generated from the schema snapshot, not hand
  invented independently.
- A2UI Basic is a baseline, not the complete Capsem product surface.
- Capsem needs extended artifacts beyond A2UI Basic: table/sheet, charts,
  diagram, timeline, generated media, slides, slide decks, spreadsheet export,
  and document export.
- Preline markup/classes should be treated as real templates, not vibes. The UI
  renderer must use the actual Preline component patterns and Capsem semantic
  tokens.

### Chat and workspace stream

The useful runtime contract is append-oriented and replayable:

```text
id
timestamp
principal
role
verb
content_type
title
content
```

Important correction:

- "kind" was too overloaded. Use `content_type` for the content payload family.
- `title` is needed because objects need display names.
- The UI should not directly mutate random Svelte state. It should replay a
  durable stream into a projection.
- Replace/edit must be by stable element/artifact/topology ids, not by vague
  "regions."
- A checkpoint mechanism is needed to collapse/prune older messages and keep
  local storage/replay from becoming cluttered.

### CRDT/editable state decision

We compared Yjs/Yrs, Automerge, and Loro at the design level for editable
artifacts. Current selected path:

- Use a Capsem authoritative record envelope for events, commands, permissions,
  and replay.
- Use Loro for editable artifact state where collaborative nested updates
  matter.
- Do not use all CRDT libraries. Keep one codebase and share as much logic as
  possible.
- All cards/artifacts should eventually be editable/configurable, not only
  spreadsheets or slides.

Why Loro for now:

- Rust-first enough for the server side.
- Better fit than Yjs for "same logic shared in Rust and TS" direction.
- Simpler than carrying multiple CRDT engines.
- Needs more testing for slides, spreadsheets, and full HTML/card editing before
  production.

### Export architecture

Important correction:

- Rust should be the typed control plane, validator, job runner, and audit wall.
- Do not implement office formats in Rust unless absolutely necessary.
- For office/PDF export, use VM-packaged tools such as LibreOffice/OpenOffice,
  Python libraries (`openpyxl`, `python-pptx`, `xlsxwriter`), Chromium print,
  Pandoc, or equivalent installed tooling.
- Plotly and Mermaid are different because their live UI rendering is part of
  the contract. Chart/diagram export should reuse the authoritative browser
  renderer or a parity-checked headless browser path.

Current export state:

- XLSX export is real enough for the spike through `openpyxl`.
- Deck HTML export is real enough for the spike.
- PPTX/PDF export currently returns typed `toolUnavailable` until the office
  toolchain is installed/configured.
- This is acceptable for the spike but not acceptable for product completion.

### Telemetry correction

Telemetry is not a first-order blocker for integrating this lane.

Reason:

- MCP/model/tool actions are already captured by Capsem/hypervisor plumbing.
- The prototype needs local operational state only: export job status, file refs,
  render/export errors returned to the caller, and maybe focused debug timings.
- Do not list "production telemetry integration" as a primary blocker unless a
  concrete missing data path is discovered.

## Prototype Lanes and File Map

### 1. WASM plugin engine

Core files:

```text
crates/capsem-plugin-engine/src/lib.rs
crates/capsem-plugin-engine/examples/compiler_spike.rs
crates/capsem-plugin-server/src/main.rs
test/compile-run.test.ts
```

What exists:

- Placeholder plugin runtime.
- Wasmtime WAT runtime path.
- AssemblyScript/TypeScript-to-WASM compiler spike in the example harness.
- Runtime install/run flow.
- BLAKE3 artifact addressing.
- Fuel-budgeted Wasmtime path.
- Memory limits.
- Host output/response ABI.
- Filesystem ABI.
- Fetch ABI.
- UI emit ABI.
- Guest abort compatibility.
- Callback dispatch metadata.

What it proves:

- Rust-only runtime path is much faster and cleaner than the earlier Node HTTP
  compiler path.
- Load/compile/instantiate/run can be benchmarked in Rust.
- The direction is viable for isolated plugin callbacks.

What is not done:

- Production TS authoring SDK.
- Stable AssemblyScript extension story.
- Out-of-process runtime isolation.
- Stable plugin manifest/market contract.
- Full Capsem security object/context integration.
- Mainline Capsem integration.

Rough integration readiness:

- TypeScript-to-WASM plugin system: about 25-35 percent.
- Runtime spike itself: useful, but not production-boundary complete.

### 2. Rust plugin server and local tool host

Core file:

```text
crates/capsem-plugin-server/src/main.rs
```

Main server roles:

- Hosts old plugin install/run routes.
- Hosts native/local MCP-shaped routes for workspace, UI, data, generation, and
  export.
- Serves exported files.
- Serves the UI preview routes.

Important routes:

```text
/plugins/install
/plugins/run
/plugins/install-run

/native/mcp/tools

/native/workspace/reset
/native/workspace/snapshot
/native/workspace/stream
/native/workspace/checkpoint
/native/workspace/info
/native/workspace/change-request
/native/workspace/resolve
/native/workspace/mutate
/native/workspace/select
/native/workspace/title
/native/workspace/style
/native/workspace/delete

/native/data/sqlite/query
/native/data/sheet

/native/generate/text
/native/generate/image
/native/generate/embedding

/native/ui/table
/native/ui/chart
/native/ui/diagram
/native/ui/timeline
/native/ui/slide
/native/ui/slide-deck
/native/ui/render-artifact
/native/ui/render-error

/native/export/spreadsheet
/native/export/slide-deck
/native/export/files/...
```

Tool names currently exposed in descriptors include:

```text
local.ui.info
local.ui.render
local.ui.comment
local.ui.resolve
local.ui.mutate
local.ui.table
local.ui.chart
local.ui.timeline
local.ui.slide
local.ui.slideDeck
local.generate.text
local.generate.image
local.generate.embedding
local.export.spreadsheet
local.export.slideDeck
```

Deferred/named but not complete:

```text
local.ui.diagram export parity
local.export.chart
local.export.diagram
local.export.pdf
```

Status:

- Good prototype local tool host.
- Not wired into real Capsem MCP yet.
- Needs route schemas and descriptor parity hardened before another agent ports
  it.

### 3. Typed UI catalogue and native artifacts

Core files:

```text
crates/capsem-ui-catalog/src/lib.rs
crates/capsem-ui-catalog/src/native_deck.rs
crates/capsem-ui-catalog/src/contract_matrix.rs
crates/capsem-ui-catalog/tests/native_deck.rs
crates/capsem-ui-catalog/tests/contract_matrix.rs
crates/capsem-ui-catalog/tests/ui_tools.rs
schemas/capsem-ui/artifacts/native-artifact.v1.schema.json
scripts/generate-native-artifact-types.ts
ui-preview/src/generated/nativeArtifact.ts
python/capsem_native_client/generated_native_artifact.py
```

Artifact families:

- A2UI Basic baseline.
- Alert.
- Ask/inline ask.
- Card.
- Status/callout.
- Table/sheet.
- Chart.
- Diagram.
- Timeline.
- Generated text.
- Generated image.
- Generated embedding.
- Slide.
- Slide deck.

Chart requirements captured:

- bar chart
- line chart
- heatmap
- box plot
- scatter
- multi-series
- stacked and unstacked
- vertical and horizontal direction
- fit/function overlay
- dual/secondary axis
- labels, units, legends
- export to PNG/SVG/PDF eventually

Status:

- Rust artifact model and schema generation exist.
- TS and Python bindings are generated from the schema.
- The contract matrix freezes what must be rendered/toolable.
- More primitives are still missing from the full desired catalogue.

### 4. Workspace stream, topology, comments, and editable state

Core files:

```text
crates/capsem-ui-catalog/src/workspace.rs
crates/capsem-ui-catalog/src/editable_state.rs
crates/capsem-ui-runtime/src/lib.rs
crates/capsem-ui-catalog/tests/workspace.rs
crates/capsem-ui-catalog/tests/editable_state.rs
test/loro-state.test.ts
test/artifact-topology.test.ts
test/native-workspace.test.ts
ui-preview/src/elements/topologyRegistry.ts
```

What exists:

- Workspace records.
- Stream projection.
- Snapshot/replay.
- Checkpoints.
- Task/comment model.
- Selection and topology id handling.
- Text/title/style/delete mutations.
- Render error records.
- Basic Loro editable-state fixture.
- Rust-to-TS topology parity checks.

Important UX decisions:

- User feedback must flow from UI to agent.
- Comment mode should let the user select a precise DOM/topology target.
- Comments should stay visible until resolved.
- Clicking a comment should reopen the existing comment, not an empty box.
- Resolving should turn into a checkmark and then disappear from active view.
- There must be a task list showing open/resolved feedback.
- Selection must be generic DOM/topology traversal, not per-component if-cases.

Status:

- Good prototype.
- Needs much stronger per-element topology editing before product.
- Needs local storage/reconnect/reconciliation hardening.

### 5. Svelte/Preline preview UI

Core files:

```text
ui-preview/src/ChatPage.svelte
ui-preview/src/A2Node.svelte
ui-preview/src/nativeDeck.ts
ui-preview/src/elements/capsem-elt.ts
ui-preview/src/elements/capsem-artifacts.ts
ui-preview/src/elements/index.ts
ui-preview/src/elements/topologyRegistry.ts
```

What exists:

- `/chat` prototype shell.
- Theme switcher and dark mode switch.
- Chat stream projection.
- Workspace replay and reconnect handling.
- Generated cards/images.
- Table/sheet rendering.
- Plotly chart rendering.
- Mermaid diagram rendering.
- Timeline rendering.
- Slide and slide deck rendering.
- Comment/annotation overlay prototype.
- Shadow DOM based `<capsem-*>` components.

Important correction:

- The UI must use actual Preline component patterns and semantic tokens.
- Earlier components were too improvised. Keep tightening toward real Preline
  classes/templates.
- Shadow DOM components need token propagation/CSS variable handling so the
  components do not become black/unreadable or drift from theme.

Status:

- Good prototype shell.
- Not production Capsem UI.
- Must be ported carefully into `/Users/elie/.codex/worktrees/2536/capsem/frontend`
  once contracts are locked.

### 6. Capsem AI/provider wrapper and generation

Core files:

```text
crates/capsem-ai/src/lib.rs
crates/capsem-plugin-server/src/main.rs
```

What exists:

- Capsem-owned provider wrapper prototype.
- Text/image/embedding generation routes.
- Environment/config based initialization path.
- Usage/cost metadata direction captured.

Important decisions:

- The crate name direction is `capsem-ai`, not `capsem-generation`.
- We looked at external Rust LLM crates and rejected direct dependency use for
  this lane. The useful idea is a clean provider API surface, not importing a
  risky crate implementation wholesale.
- This code path must be reusable by dynamic security review later, not just UI
  generation.
- It must wire to Capsem settings eventually, where the Gemini key already
  exists.
- It should support model usage accounting: tokens, cost, latency, provider,
  model, and response id where available.

Status:

- Prototype exists.
- Actual model names and current provider APIs must be verified against the
  live provider docs/settings before locking.

### 7. Python native client

Core files:

```text
python/capsem_native_client/__init__.py
python/capsem_native_client/generated_native_artifact.py
python/tests/test_capsem_native_client.py
```

Purpose:

- Rehearsal client for the eventual local MCP surface.
- Lets an agent call the Rust webserver through a small Python API before the
  final MCP adapter is written.

Namespaces:

```text
native
workspace
data.sqlite
generate
ui
export
```

Status:

- Useful for tool/API rehearsal.
- Must remain a thin client. The Rust server is still the contract authority.

### 8. Export and document toolchain

Core files:

```text
scripts/export_artifact.py
scripts/e2e-export.ts
crates/capsem-plugin-server/src/main.rs
```

What exists:

- XLSX export through `openpyxl`.
- XLSX workbook inspection in the E2E proof.
- Chart sheet embedding for simple sheet/chart relation.
- Deck HTML export.
- File serving for exported artifacts.
- Typed `toolUnavailable` status for PPTX when the needed toolchain is missing.

Known local dependency state:

- Bundled Python has `openpyxl 3.1.5`.
- System Python did not have the same packages during the spike.
- Bundled Python path used by tests:

```text
/Users/elie/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3
```

- `python-pptx`, `xlsxwriter`, `plotly`, and `kaleido` were not present in the
  bundled Python when checked.

Product requirement:

- User must be able to dictate a slide deck one slide at a time, get a real
  slide deck file out, and upload/open it elsewhere.
- Spreadsheets and charts must be exportable.
- PDF story is still missing.
- This should use VM-packaged tools where possible rather than reimplementing
  office formats.

Status:

- XLSX and deck HTML: working spike.
- PPTX/PDF: not done.
- Plotly/Mermaid export: not done; should use browser renderer parity path.

## Sprint Documents

Existing sprint directories in the prototype:

```text
sprints/plugin-event-prototype/
sprints/ts-compiler-spike/
sprints/rust-plugin-engine-spike/
sprints/js-wasm-component-spike/
sprints/plugin-install-registry/
sprints/mini-capsem-file-ui-plugin/
sprints/a2ui-preline-template-system/
sprints/capsem-element-foundation/
sprints/chat-shell-ui-surface/
sprints/chat-theme-switcher/
sprints/ui-catalogue-pack-01/
sprints/ui-catalog-api-preview/
sprints/ui-authoring-live-lane/
sprints/ui-extension-contract-research/
sprints/ui-table-render-contract/
sprints/ui-ask-inline-block/
sprints/ui-catalogue-split/
sprints/hyper-a2ui-contract-lane/
sprints/generation-provider-lane/
sprints/capsem-ai-accounting-lane/
sprints/capsem-native-mcp-and-deck-proof/
sprints/workspace-stream-projection/
sprints/mainline-ui-runtime-integration/
```

Most important current sprint folder:

```text
sprints/mainline-ui-runtime-integration/
```

Key files there:

```text
MASTER.md
tracker.md
plan.md
artifact-catalogue.md
selected-protocol.md
porting.md
```

Use that folder to orient the next integration-focused sessions, but update it
to reflect the telemetry correction and export architecture correction from this
summary.

## Verification Commands

Run from:

```text
/Users/elie/.codex/worktrees/2536/capsem-plugin-prototype
```

Focused Rust tests:

```bash
cargo test -p capsem-plugin-server
cargo test -p capsem-ui-catalog --test contract_matrix
```

Python client test:

```bash
PYTHONPATH=python /Users/elie/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3 -m unittest python.tests.test_capsem_native_client
```

Schema/binding parity:

```bash
npm run check:artifact-types
```

Chat/browser E2E:

```bash
npm run e2e:chat
```

Workspace session inspection:

```bash
npm run inspect:workspace
```

Export E2E:

```bash
npm run inspect:export
```

TypeScript and whitespace:

```bash
npm run typecheck
git diff --check
```

Workspace benchmark:

```bash
npm run bench:workspace
```

Latest known good checks before this summary:

```text
npm run inspect:export  -> passed
npm run typecheck       -> passed
git diff --check        -> passed
```

The export E2E pass included:

```text
XLSX export produced a downloadable workbook
Workbook rows: 4
Workbook columns: 3
House rows included Stark, Lannister, Targaryen
Workbook included a chart sheet
Deck HTML export produced a downloadable HTML file
PPTX export returned typed toolUnavailable
Workspace/export statuses reconciled through the server
```

Earlier focused checks also passed:

```text
cargo test -p capsem-plugin-server
cargo test -p capsem-ui-catalog --test contract_matrix
Python client unittest through bundled Python
direct export_artifact.py openpyxl smoke
```

## Mainline Integration Readiness

### UI/runtime/tooling lane

Current estimate:

```text
About 60 percent toward an isolated feature lock.
About 35-45 percent toward safe mainline integration.
```

Why not higher:

- Mainline Svelte/Preline port has not happened.
- Real Capsem MCP adapter has not been written.
- Export path is incomplete for PPTX/PDF and chart/diagram exports.
- Comment/topology editing is promising but not production-grade.
- Preline component parity is not finished.
- Shadow DOM/theme/token handling still needs hardening.
- Selector/path/file-serving security needs production review.

Why not lower:

- Rust contracts exist.
- Generated TS/Python bindings exist.
- Server routes exist.
- Chat UI proof exists.
- Workspace stream/replay/checkpoint exists.
- Export spike exists.
- E2E scripts exercise real flows.

### WASM/plugin lane

Current estimate:

```text
About 25-35 percent toward the full TypeScript-to-WASM plugin system.
```

Why:

- Wasmtime runtime spike works.
- ABI patterns exist.
- Compiler spike exists.
- But authoring SDK, compiler discipline, manifest, security isolation,
  out-of-process execution, and Capsem object integration are not locked.

## What Is Left Before Mainline Port

Priority order:

1. Lock the UI/runtime contract.
2. Port the renderer to the real Capsem frontend and real Preline tokens.
3. Wire the local tool surface into real Capsem MCP naming, permissions, and
   routing.
4. Finish export toolchain for XLSX, PPTX, PDF, Plotly chart export, Mermaid
   export, and file download/open flows.
5. Prove agent-only acceptance: user asks for a slide deck, agent creates
   image/table/chart/diagram/slides through tools, user edits with comments,
   agent applies precise mutations, final deck exports.
6. Harden selector/topology security, file serving, renderer sandboxing, and
   external command execution.
7. Return to WASM plugin runtime and bind it to the now-stable UI/tool
   contracts.

Not a blocker:

- Production telemetry integration, unless a concrete missing signal is found.
  Existing Capsem MCP/model/hypervisor capture should carry that work.

## Suggested Focus Sessions

### Session A: Mainline UI port map

Goal:

- Read `/Users/elie/.codex/worktrees/2536/capsem/frontend`.
- Map prototype `ui-preview` pieces to real frontend routes/components/tokens.
- Produce a porting plan with exact target files and missing Preline/token work.

Inputs:

```text
ui-preview/src/ChatPage.svelte
ui-preview/src/A2Node.svelte
ui-preview/src/elements/
sprints/mainline-ui-runtime-integration/porting.md
```

Output:

- Mainline port plan only, unless explicitly asked to implement.

### Session B: Real Capsem MCP adapter

Goal:

- Map `/native/mcp/tools` descriptors to the real Capsem MCP host.
- Decide final naming hierarchy, likely `local.ui.*`, `local.generate.*`,
  `local.data.*`, `local.export.*`.
- Make sure tools run inside Capsem's sandbox/tooling model.

Inputs:

```text
crates/capsem-plugin-server/src/main.rs
python/capsem_native_client/__init__.py
crates/capsem-ui-catalog/src/contract_matrix.rs
```

Output:

- Adapter design or implementation spike.

### Session C: Office export toolchain

Goal:

- Add/install/configure the VM-assisted export tools.
- Implement PPTX export.
- Add PDF export.
- Keep Rust as typed job runner, not document-format engine.

Likely tools:

```text
LibreOffice/OpenOffice
python-pptx
openpyxl
xlsxwriter
Chromium print
Pandoc, if useful
```

Inputs:

```text
scripts/export_artifact.py
scripts/e2e-export.ts
```

Output:

- Real XLSX/PPTX/PDF artifacts with E2E tests.

### Session D: Browser renderer export for Plotly and Mermaid

Goal:

- Export chart/diagram using the same renderer contract seen by the UI.
- Avoid divergent VM-side renderers for Plotly/Mermaid.
- Produce PNG/SVG/PDF where feasible.

Inputs:

```text
ui-preview/src/elements/capsem-artifacts.ts
scripts/e2e-export.ts
crates/capsem-ui-catalog/src/native_deck.rs
```

Output:

- Headless browser export path and parity tests.

### Session E: Comment/topology editing hardening

Goal:

- Make user feedback precise and durable.
- Support selecting any artifact element, card field, table row/cell, chart
  region where possible, slide region, and generated media.
- Let the user type feedback in a small floating panel.
- Keep markers visible until resolved.
- Feed comments back into the tool/agent workflow.

Inputs:

```text
ui-preview/src/ChatPage.svelte
ui-preview/src/elements/topologyRegistry.ts
crates/capsem-ui-catalog/src/workspace.rs
```

Output:

- Better annotation UX and tests.

### Session F: WASM plugin hardening

Goal:

- Resume the plugin runtime after the UI/tool contracts are less fluid.
- Add out-of-process execution.
- Stabilize manifest.
- Stabilize callback objects and context.
- Decide AssemblyScript/TS compiler constraints.
- Bind plugin UI intrinsics to the same typed artifact/tool surface.

Inputs:

```text
crates/capsem-plugin-engine/src/lib.rs
crates/capsem-plugin-engine/examples/compiler_spike.rs
crates/capsem-plugin-server/src/main.rs
sprints/rust-plugin-engine-spike/
sprints/ts-compiler-spike/
```

Output:

- Runtime architecture decision and next implementation sprint.

### Session G: Catalogue and Preline parity

Goal:

- Expand the component catalogue.
- Use real Preline component structures.
- Ensure every typed UI component has renderer support and tests.
- Fail tests if a typed UI component has no renderer.

Inputs:

```text
templates/capsem-ui/
private/todo/
crates/capsem-ui-catalog/src/contract_matrix.rs
ui-preview/src/A2Node.svelte
ui-preview/src/elements/capsem-artifacts.ts
```

Output:

- Stronger catalogue with render parity gates.

## Known Gotchas

- Do not confuse "A2UI Basic" with the full Capsem UI system.
- Do not add arbitrary JSON dictionaries where typed enums/objects are needed.
- Do not invent "weather card" or other non-catalogue components without adding
  a typed contract and renderer.
- Do not treat `emit` as an API noun. User-facing API should be explicit:
  `ui.alert(...)`, `ui.ask(...)`, `ui.card(...)`, `ui.table(...)`,
  `ui.chart(...)`, etc.
- Do not bypass the workspace stream by directly editing Svelte state for core
  artifact changes.
- Do not treat a successful prototype route as production-ready MCP.
- Do not use code defaults for provider models. Model/provider settings must
  come from config/settings.
- Do not assume old Gemini model names are current. Verify against current
  provider docs or Capsem settings before locking.
- Do not implement full office formats in Rust unless the external tool route is
  impossible.
- Do not make chart/diagram export diverge from the live UI renderer.
- Do not port to mainline until the rest is locked, fully tested, and
  benchmarked.

## Fast Reorientation Recipe

Use this when opening a fresh focus session:

```bash
cd /Users/elie/.codex/worktrees/2536/capsem-plugin-prototype
git status --short --branch
sed -n '1,260p' summary.md
sed -n '1,260p' sprints/mainline-ui-runtime-integration/MASTER.md
sed -n '1,260p' sprints/mainline-ui-runtime-integration/tracker.md
```

Then pick the lane:

- WASM/plugin: start with `crates/capsem-plugin-engine/src/lib.rs`.
- Tool host: start with `crates/capsem-plugin-server/src/main.rs`.
- UI contracts: start with `crates/capsem-ui-catalog/src/native_deck.rs`.
- Workspace/editing: start with `crates/capsem-ui-catalog/src/workspace.rs`.
- Svelte renderer: start with `ui-preview/src/ChatPage.svelte` and
  `ui-preview/src/elements/capsem-artifacts.ts`.
- Export: start with `scripts/export_artifact.py` and `scripts/e2e-export.ts`.
- Python tool client: start with `python/capsem_native_client/__init__.py`.

## Current Bottom Line

We have two separate efforts that should remain mentally distinct:

1. The WASM plugin system is promising but still a runtime/compiler/security
   spike.
2. The typed UI/runtime/MCP/export system is much closer to an isolated
   shippable feature, but still needs contract lock, Preline parity, export
   completion, and mainline integration work before it can land.

The next clean move is not to merge. The next clean move is to split the work
into focused sessions, finish the UI/runtime/tool/export contract to a hard
acceptance gate, and only then wire it into real Capsem.
