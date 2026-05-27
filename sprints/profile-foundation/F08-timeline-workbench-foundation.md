# F08 - Timeline And Workbench Foundation

## Goal

Build the everyday-work timeline/workbench on top of canonical resolved events.

## Scope

- Conversation Engine and structured `/timeline/{id}` API.
- Timeline blocks for prompts, responses, tools, files, network, processes,
  findings, asks/confirms, snapshots, artifacts, and profile/rule provenance.
- Codex/Claude SDK-backed sessions and terminal fallback.
- Search, filtering, and review workflows over the same event ids.

## Acceptance Criteria

- Workbench never reads raw legacy logs as authority when resolved events exist.
- Timeline rows link back to canonical event ids and session/profile identity.
- Real session proof covers at least one AI/tool/file/network path.
