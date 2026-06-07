# Sprint 04: Settings System

Design and implement the full settings architecture with persistence.

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
Depends on: Sprint 01

## Acceptance Criteria

### Settings Store
- [x] `settings.svelte.ts` — rune-based settings store
- [x] `load()` — initialize from localStorage (appearance) + defaults (everything else)
- [x] `stage(section, key, value)` — local change without persisting
- [x] `save()` — persist staged changes
- [x] `reset(section)` — revert section to defaults
- [x] Type-safe settings model with defaults

### Settings Sections
- [x] **Appearance** — dark/light toggle + 9 Preline theme picker (refine existing)
- [x] **General** — VM defaults: RAM (slider/input), CPUs (slider/input), timeout
- [x] **Security** — allowed domains list (add/remove), MCP server allowlist
- [x] **Network** — proxy config (host, port, auth), port forwarding rules
- [x] **Storage** — default image selector, workspace path
- [x] **Advanced** — debug logging toggle, experimental features toggles
- [x] **About** — version info, links to docs/repo

### UI Components
- [x] Refine `SettingsPage.svelte` with real form controls (not placeholder text)
- [x] Section components in `settings/` subdirectory
- [x] Form validation (RAM range, valid ports, valid domains)
- [x] Unsaved changes indicator + save/discard buttons

### Persistence
- [x] Appearance settings persist via localStorage (immediate, no gateway needed)
- [x] Other settings: stage locally, persist via gateway API in Sprint 05
- [x] Settings export/import (JSON)

## Testing Gate

- [x] All settings sections render with functional form controls
- [x] Theme/mode changes persist across page reload
- [x] Settings store unit tests: stage, save, reset, load
- [x] Form validation prevents invalid values
- [x] Chrome DevTools MCP screenshot of each settings section in both themes
- [x] `pnpm run check` passes
