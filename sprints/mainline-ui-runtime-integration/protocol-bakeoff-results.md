# Protocol Bake-Off Results

## Result

Selected protocol:

- **Capsem record envelope** for the runtime/audit/replay protocol.
- **Loro** for all editable user-visible UI and artifact state.
- **A2UI Basic** through `a2ui-types` for standard UI component wire validation and projection.
- **Capsem extension artifacts** for charts, diagrams, slides, spreadsheets,
  websites/forms, generated media, and timelines.

Simplicity rule: implement one shared protocol path. Automerge and Yrs/Yjs were
considered and rejected for this sprint. They must not be added as alternate
backends during implementation.

## Candidate Scores

| Candidate | Audit/provenance | Cards/UI | Slides | Spreadsheets | Websites/forms | Rust/TS story | Decision |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Capsem record envelope only | Excellent | Possible but we would hand-roll configurable UI state | Possible but we would hand-roll tree/list conflict behavior | Possible but we would hand-roll cell/range edits | Possible but we would hand-roll page/form edits | Excellent because owned | Use as outer protocol only |
| A2UI-native stream | Weak for audit/task/provenance | Good render shape, weak editable state | Not enough | Not enough | Good for form primitives, not full website artifact state | Good for components | Use as UI wire inside envelope |
| A2UI plus Capsem envelope | Excellent | Needs editable state engine | Needs editable state engine | Needs editable state engine | Needs editable state engine | Excellent | Use as canonical runtime shape |
| Loro document state | Needs Capsem envelope | Strong fit: cards are maps/lists/text with configurable actions | Strong fit: ordered slides, movable region tree, rich text | Strong fit: sheets/cells as maps/lists; ranges need schema | Strong fit: page/form tree and reorderable fields | Strong: Rust plus JS/WASM | Use for editable UI/artifact state |
| Automerge document state | Needs Capsem envelope | Good JSON-like document fit | Good, but tree topology is less direct | Good JSON-like document fit | Good JSON-like document fit | Strong: Rust plus JS/WASM | Rejected for this sprint; do not add |
| Yrs/Yjs document state | Needs Capsem envelope | Possible but less natural unless we lean into shared types | Possible but less natural unless we lean into XML/shared types | Possible but not primary strength | Possible; strong browser ecosystem | Strong ecosystem; Yrs Rust compatibility | Rejected for this sprint; do not add |
| MCP event log | Good for tool observations | Not stateful enough | Not stateful enough | Not stateful enough | Not stateful enough | Good for agents | Tool boundary, not canonical state |

## Why Loro Wins For UI And Artifacts

The non-negotiable day-one objects are editable cards, slides, spreadsheets, and
websites/forms. Those are structured documents:

- Cards are user-configurable maps/lists/text with actions and embedded media.
- Slide decks are ordered lists of slides.
- Slides are trees/lists of regions.
- Spreadsheets are workbooks of sheets, cells, ranges, and embedded artifacts.
- Websites are page/section/form trees with reorderable fields and validation
  text.

Loro's maps, lists, text, and movable trees match that structure directly. It
also gives us Rust and JavaScript/WASM support, which is mandatory for a Rust
runtime and Svelte/browser renderer.

## Why The CRDT Is Not The Whole Protocol

The CRDT cannot be the security/audit protocol. Capsem records must wrap any
artifact update so we can track:

- principal
- role
- timestamp
- permission result
- source tool/model/plugin
- model provider and cost
- telemetry timing
- mutation target
- rejection reason

That preserves the security posture while still giving editable UI/artifacts a
real collaborative state model.

## Day-One Protocol Families

These are in the first protocol contract:

- `ui.surface`
- `ui.component.*`
- `artifact.card`
- `artifact.chart.*`
- `artifact.diagram.mermaid`
- `artifact.spreadsheet`
- `artifact.sheet`
- `artifact.slideDeck`
- `artifact.slide`
- `artifact.website`
- `artifact.page`
- `artifact.form`
- `artifact.image`
- `artifact.video`
- `artifact.audio`
- `artifact.timeline`
- `ui.comment`
- `ui.task`
- `artifact.*.mutation`
- `workspace.checkpoint`
- `tool.call`
- `tool.observation`

## Loro API Audit

Status: passed for the spike.

The checked fixture lives in:

- `crates/capsem-ui-catalog/src/editable_state.rs`
- `crates/capsem-ui-catalog/tests/editable_state.rs`
- `test/loro-state.test.ts`

It verifies:

- Rust crate API quality for maps and movable lists is sufficient for the first
  editable UI/artifact contract.
- `loro-crdt` browser package import/export works under the TypeScript test
  environment.
- Card edit/configuration updates work through Loro.
- Stable Capsem topology ids survive Loro list moves.
- Snapshot plus incremental update can be wrapped in a Capsem-style record and
  replayed after restore.

Measured Rust fixture:

| Measure | Value |
| --- | ---: |
| Base snapshot | 2,181 bytes |
| Incremental update | 196 bytes |
| Updated snapshot | 2,304 bytes |

Dependency note: `loro` pulled a non-trivial Rust dependency tree. Keep that in
the production dependency audit, but do not add Automerge or Yrs/Yjs unless the
protocol decision is formally reopened.
