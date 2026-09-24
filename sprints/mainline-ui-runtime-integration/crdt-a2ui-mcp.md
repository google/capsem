# CRDT, A2UI, And MCP Contract Decision Note

## Why This Exists

The prototype proved the UI projection lane, but production needs harder discipline before mainline integration. First we test protocol alternatives; then we build the selected protocol.

- The workspace stream/checkpoint model may overlap with existing CRDT systems.
- The current Capsem `ui.*` helpers expose only a subset of A2UI Basic.
- The MCP/tool surface discussed in design is not yet fully enumerated or tested.

This note turns those into sprint gates. Rich artifacts are included because the
same runtime must carry slides, spreadsheets, websites/forms, generated media,
charts, diagrams, and timelines from the first protocol.

See `protocol-bakeoff.md` for the required candidate fixture and measurements.

## CRDT Candidates

| Option | Rust story | TS/browser story | Fit for Capsem UI runtime | Main risk |
| --- | --- | --- | --- | --- |
| Capsem append log plus checkpoints | Native and fully auditable | Svelte consumes projected frames | Best for security/audit, principal tracking, deterministic replay, and single-authoritative-session semantics | We must build conflict/reconnect behavior ourselves if multi-writer/offline becomes real |
| Yjs plus Yrs | `yrs` is a Rust port intended to interoperate with Yjs binary protocol | Yjs is mature, widely used, and has providers/bindings | Strong candidate if we want browser-local offline edits, shared cursors, or multi-client collaboration | Adds CRDT document semantics and provider complexity to a security-sensitive runtime |
| Automerge | Rust core with JS/WASM bindings and a JSON-like document model | Good local-first story and sync protocol | Strong candidate if JSON document history and offline merges matter more than A2UI-specific structure | Must benchmark memory/history cost and prove Rust/browser isomorphism for our schema |
| Loro | Rust-first, JS via WASM, supports maps/lists/text/trees and version history | Good local-first API, promising tree support | Interesting if UI topology becomes a collaborative mutable tree | Younger ecosystem than Yjs/Automerge; larger/newer dependency surface to audit |
| JSON Joy / Diamond Types style libraries | Mixed; often TS-first or text/list-focused | Good performance research and benchmarks | Reference points for performance, less likely as first production substrate | May not give us the Rust+TS audited object model we need |

Decision after the bake-off and API audit: keep Capsem's authoritative append
log as the production spine, and use Loro for every editable user-visible
UI/document state. A2UI-plus-Capsem-envelope remains the runtime shape; A2UI is
projection, not the editable state store.

The Loro audit fixture passed in Rust and browser JS with a card, alert, slide
deck, sheet, and website form. The measured Rust fixture produced a 2,181-byte
base snapshot, 196-byte incremental update, and 2,304-byte updated snapshot.

## CRDT Spike Acceptance

The spike must model the same document in each viable candidate:

- A workspace with ordered records.
- A component topology tree.
- A comment/task ledger.
- A generated artifact reference.
- A title/text/style mutation.
- A checkpoint or snapshot equivalent.

The proof must measure:

- Rust create/apply/update cost.
- Browser apply/update cost.
- Serialized update size.
- Restart restore behavior.
- Deterministic projection from the same logical changes.
- Conflict behavior for two edits to the same component title and two comments on the same sub-element.
- How principal, timestamp, provenance, and telemetry attach to each change.

If no candidate handles provenance/audit cleanly, the sprint keeps Capsem append-log canonical and may use CRDT only as an optional collaborative editor layer later.

## A2UI Current State

Local A2UI Basic v0.9 schema includes:

- `Text`
- `Image`
- `Icon`
- `Video`
- `AudioPlayer`
- `Row`
- `Column`
- `List`
- `Card`
- `Tabs`
- `Modal`
- `Divider`
- `Button`
- `TextField`
- `CheckBox`
- `ChoicePicker`
- `Slider`
- `DateTimeInput`

Current Capsem helper layer is narrower:

- `ui.alert`
- `ui.notice`
- `ui.card`
- `ui.facts`
- `ui.table`
- `ui.ask`

That is acceptable only if we name it as Pack 01. It is not acceptable to call this full A2UI coverage.

## A2UI Translation Gates

Every shipped high-level helper must pass three gates:

1. Call-to-wire: `ui.card(...)` or equivalent typed call lowers to valid A2UI Basic or a documented Capsem catalog extension.
2. Wire-to-render: every emitted component kind has an explicit Svelte/Preline renderer.
3. Render-to-topology: every user-addressable subpart has a stable topology id for `local.ui.info`, comments, and mutations.

The test rule:

- A helper without A2UI validation must fail.
- A component emitted without a renderer must fail.
- A renderer without schema/catalog coverage must fail.
- A topology id missing from `ui.info` must fail.
- A mutation target that can only be addressed by incidental CSS selector must fail for production components.

## A2UI Pack Roadmap

The machine-readable source for this boundary is
`crates/capsem-ui-catalog/src/contract_matrix.rs`.

Pack 01 remains:

- alert/notice
- card with image/link/actions
- inline ask/comment task
- facts
- table with search/filter/pagination

Pack 02 should cover the missing Basic primitives needed for real product UI:

- modal
- tabs
- text field
- checkbox
- choice picker
- slider
- date/time input
- list
- video
- audio player
- divider
- icon variants

Pack 03 should cover Capsem extensions:

- chart with Plotly-backed variants
- spreadsheet
- sheet
- website
- page
- form
- diagram
- slide
- slide deck
- timeline
- image
- video
- audio
- generated media card
- security decision panel
- plugin/tool installation card

## MCP/Tool Surface Gap

The production local tools need to be explicit. Proposed hierarchy:

- `local.ui.info`: return surfaces, topology, selected comments/tasks, and mutation affordances.
- `local.ui.render`: render a UI surface projected from checked Loro-backed UI state into A2UI/Capsem component specs.
- `local.ui.comment`: attach user/model feedback to a topology target.
- `local.ui.resolve`: resolve a comment/task.
- `local.ui.mutate`: apply typed mutations to title/text/props/style within the allowlist.
- `local.workspace.snapshot`: return compact workspace state.
- `local.workspace.stream`: subscribe/replay workspace records.
- `local.workspace.checkpoint`: compact prior records into a replay-safe checkpoint.
- `local.generate.text`: generate text through `capsem-ai`.
- `local.generate.image`: generate image through `capsem-ai`.
- `local.generate.audio`: future, provider-gated.
- `local.generate.video`: future, provider-gated.
- `local.data.sqlite`: per-instance scratch database for agent/tool workflows.
- `local.data.spreadsheet`: workbook-level spreadsheet creation/update surface.
- `local.data.sheet`: sheet-level creation/update surface.
- `local.website.render`: website/page preview surface.
- `local.website.form`: form creation/update and validation surface.
- `local.diagram.render`: diagram creation, likely Mermaid first unless we choose another renderer.
- `local.web.preview`: browser/website preview surface, separate from component/deck rendering.

Every tool must call the Rust runtime. No tool may write directly into Svelte state or raw DOM.

## Mainline Decision Gate

Before porting code to mainline, T0A must answer:

- Which protocol is canonical for records, replay, comments, mutations, artifacts, and tools?
- Which state substrate is canonical inside that protocol?
- Which A2UI Basic primitives are in Pack 01 versus deferred?
- Which Capsem extension components are allowed in the first PR?
- Which `local.*` tools ship with the first integration?
- Which tests prove call-to-wire-to-render-to-topology is complete?
- Which artifact families ship now, and which are schema-only/deferred?
