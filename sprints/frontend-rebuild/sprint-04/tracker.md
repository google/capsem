# Sprint 04: Settings System

Design and implement the full settings architecture with persistence.

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
Depends on: Sprint 01

## Acceptance Criteria

### Settings Store
- [ ] `settings.svelte.ts` — rune-based settings store
- [ ] `load()` — initialize from localStorage (appearance) + defaults (everything else)
- [ ] `stage(section, key, value)` — local change without persisting
- [ ] `save()` — persist staged changes
- [ ] `reset(section)` — revert section to defaults
- [ ] Type-safe settings model with defaults

### Settings Sections
- [ ] **Appearance** — dark/light toggle + 9 Preline theme picker (refine existing)
- [ ] **General** — VM defaults: RAM (slider/input), CPUs (slider/input), timeout
- [ ] **Security** — allowed domains list (add/remove), MCP server allowlist
- [ ] **Network** — proxy config (host, port, auth), port forwarding rules
- [ ] **Storage** — default image selector, workspace path
- [ ] **Advanced** — debug logging toggle, experimental features toggles
- [ ] **About** — version info, links to docs/repo

### UI Components
- [ ] Refine `SettingsPage.svelte` with real form controls (not placeholder text)
- [ ] Section components in `settings/` subdirectory
- [ ] Form validation (RAM range, valid ports, valid domains)
- [ ] Unsaved changes indicator + save/discard buttons

### Persistence
- [ ] Appearance settings persist via localStorage (immediate, no gateway needed)
- [ ] Other settings: stage locally, persist via gateway API in Sprint 05
- [ ] Settings export/import (JSON)

## Testing Gate

- [ ] All settings sections render with functional form controls
- [ ] Theme/mode changes persist across page reload
- [ ] Settings store unit tests: stage, save, reset, load
- [ ] Form validation prevents invalid values
- [ ] Chrome DevTools MCP screenshot of each settings section in both themes
- [ ] `pnpm run check` passes
