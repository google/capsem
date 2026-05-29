# UI API Design Research Swarm

## Purpose

Design the author-facing UI API shape for Capsem extensions and model-generated
UI. This swarm is about API design vocabulary and object shape, not transport
plumbing.

Key correction: `emit` is an implementation verb. Authoring APIs should expose
nouns/surfaces such as `ui.sidePanel(...)`, `ui.modal(...)`, `ui.tab(...)`,
`ui.chat(...)`, `ui.statusItem(...)`, and renderer declarations.

## Status Legend

- Not launched
- In progress
- Completed
- Captured

## Finding Docs Index

| Status | Domain | Agent | Finding Doc | Sprint Target |
| --- | --- | --- | --- | --- |
| Captured | Chrome extension UI API | Halley / `019e7492-ffcd-7552-90a8-8c45003fa6c2` | `swarm-findings/chrome-extension-ui.md` | Manifest and surface vocabulary |
| Captured | Agent UI protocols | Laplace / `019e7493-17e4-79d3-b87e-e92c47471e6e` | `swarm-findings/agent-ui-protocols.md` | Model-generated UI object shape |
| Captured | ArrowJS authoring API | Gauss / `019e7493-35d7-7170-b62d-182d80e8281d` | `swarm-findings/arrowjs-authoring.md` | Lightweight renderer ergonomics |
| Captured | Capsem current UI/protocol fit | Cicero / `019e7493-4d61-77b2-97c8-0d0e8c60246e` | `swarm-findings/capsem-ui-fit.md` | Fit with Svelte/Preline and existing typed iframe protocol |
| Captured | Generative UI supplemental sources | local | `swarm-findings/generative-ui-supplemental.md` | A2UI, assistant-ui, CopilotKit, prompt-kit, motion-primitives, ArrowJS public docs |

## Resume Protocol

1. Read this board.
2. Read every finding doc.
3. Poll active agents.
4. Capture completed results into finding docs before relying on them.
5. Do not synthesize until every finding doc is completed or explicitly
   deferred.

## Required Finding Shape

- Source/code inspected.
- Author-facing API vocabulary.
- Object shapes and examples.
- What to copy.
- What to reject.
- Capsem implication.
- Open design questions.

## Completed Agents

- Halley / `019e7492-ffcd-7552-90a8-8c45003fa6c2` -- Chrome extension UI API.
- Laplace / `019e7493-17e4-79d3-b87e-e92c47471e6e` -- Agent UI protocols.
- Gauss / `019e7493-35d7-7170-b62d-182d80e8281d` -- ArrowJS authoring.
- Cicero / `019e7493-4d61-77b2-97c8-0d0e8c60246e` -- Capsem UI fit.

## Active Agents

None.

## Launch Queue

1. Chrome extension UI API.
2. Agent UI protocols for model-generated UI.
3. ArrowJS authoring API.
4. Capsem current UI/protocol fit.

## Intake Checklist

- [x] Agent output copied into finding doc.
- [x] Finding doc marked completed.
- [x] Board row updated.
- [x] Agent closed.

## Completeness Gate

- No active agents.
- No finding doc says `Awaiting agent output`.
- Every completed doc has clear findings.
- Synthesis distinguishes API design from host plumbing.
