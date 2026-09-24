# Selected Protocol

## Decision

Use a two-layer protocol:

1. **Capsem record envelope** is canonical for the runtime.
2. **Loro document state** is the editable model for all user-visible UI and artifact state.

Keep `a2ui-types` for A2UI Basic validation. A2UI is a projection/render wire
from the editable state, not the canonical editable state. Keep Plotly and
Mermaid as renderer backends, not state protocols.

Implementation rule: **one code path**. Do not add Automerge or Yrs/Yjs to the
codebase while implementing this decision. They are rejected alternatives, not
parallel supported backends.

## The Shape

```text
CapsemRecord {
  id,
  seq,
  principal,
  role,
  timestamp,
  contentType,
  verb,
  title,
  content,
  provenance,
  telemetry
}
```

`content` can carry:

- A2UI Basic messages.
- Capsem artifact specs.
- Loro UI/artifact snapshots or updates.
- Comments/tasks.
- Typed mutations.
- Tool calls and observations.
- Checkpoints.

The record envelope is append-only, auditable, replayable, and owns principal,
permission, telemetry, provenance, model cost, and rejection/acceptance status.

Loro owns editable state for every user-configurable visual surface:

- `card`
- `alert`
- `notice`
- `ask`
- `table`
- `facts`
- `tool result cards`
- generated media cards
- security decision cards
- `slideDeck`
- `slide`
- `spreadsheet`
- `sheet`
- `website`
- `page`
- `form`
- structured timeline

## Why Loro

All cards are editable. That means cards are not disposable render output; they
are durable user-configurable UI nodes. Slides, spreadsheets, websites, and
forms have the same property at larger scale. They need edits inside ordered and
hierarchical structures:

- Rename a card.
- Reorder card sections.
- Configure card actions.
- Patch a card's image, link, body, or badge.
- Move a slide.
- Move a slide region.
- Edit text in a slide block.
- Edit a spreadsheet cell.
- Insert/delete/reorder a row.
- Rename a sheet.
- Edit a form label.
- Reorder form fields.
- Comment on a region, range, field, label, or validation message.

Loro is the best fit because it is Rust-first, has JavaScript/WASM support, and
is built around maps, lists, text, and movable trees. Those primitives match our
UI and artifact structures directly.

## Why Not Automerge As Primary Artifact CRDT

Automerge is credible and mature for JSON-like local-first documents, with Rust
and JavaScript support. It is a rejected alternative for this sprint.

We are not choosing it first because our hard objects are topology-heavy:
cards, tables, slides, page/form trees, embedded regions, and ordered nested
structures. Loro's tree/list focus maps more directly to that work.

## Why Not Yrs/Yjs As Primary Artifact CRDT

Yrs/Yjs is excellent when we want the Yjs ecosystem: editor bindings, shared
types, awareness, providers, and browser collaboration. It is a rejected
alternative for this sprint.

We are not choosing it first because our first-class problem is not a text
editor. It is typed UI/artifact topology across cards, tables, slides, sheets,
websites, charts, diagrams, media, and forms. Yrs can model this with
maps/arrays/XML types, but the contract would be less natural than Loro's
document-tree model.

## Why Not CRDT As The Whole Protocol

Capsem cannot give up the audit spine:

- Security decisions need principal, role, timestamp, status, and reason.
- Model calls need provider, cost, token/media metadata, and latency.
- Plugin output needs permission and rejection paths.
- Users need replay, checkpoint, and inspection.
- Telemetry must explain every mutation.

CRDT updates alone are not the security protocol. They are a state representation
inside auditable records.

## Day-One Artifact Scope

No future bucket for these protocol families:

- cards and component surfaces
- slides and slide decks
- spreadsheets and sheets
- websites, pages, and forms
- images, video, and audio
- Plotly charts
- Mermaid diagrams
- timelines

Some renderers may be basic in the first implementation, but the protocol must
support all of them before mainline integration.

## Library Choices

| Role | Choice | Status |
| --- | --- | --- |
| Runtime envelope | Capsem-owned Rust types | Build |
| A2UI validation | `a2ui-types` | Use, already wired |
| Editable UI/artifact CRDT | `loro` / `loro-crdt` | Use; Rust/browser API audit passed |
| Rejected CRDT alternative | `automerge` | Do not add; revisit only if Loro fails |
| Rejected editor/ecosystem alternative | `yrs` + Yjs | Do not add; revisit only if Loro fails and editor ecosystem becomes decisive |
| Charts | Plotly | Use as renderer backend |
| Diagrams | Mermaid | Use as renderer backend |

## Loro Audit Result

The audit fixture now exists in Rust and browser JS. It models an editable card,
alert, slide deck, sheet, and website form, then mutates titles/cells/labels and
reorders lists.

Measured Rust fixture:

| Measure | Value |
| --- | ---: |
| Base snapshot | 2,181 bytes |
| Incremental update | 196 bytes |
| Updated snapshot | 2,304 bytes |

The fixture verifies:

- Rust snapshot plus update restores to the updated document.
- Browser `loro-crdt` snapshot plus update restores to the updated document.
- Stable Capsem topology ids are explicit fields in the document and survive
  list moves.
- A Capsem-style record can wrap the Loro update with principal, verb, target,
  affected ids, update size, and BLAKE3 identity.

Production caution: `loro` brings a real dependency tree. That is acceptable for
the single selected CRDT path, but it should remain visible in dependency audit
and release review.
