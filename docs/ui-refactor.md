# UI Refactor: DaisyUI + Tailwind Adoption

Companion to `docs/design.md` (design system spec) and `docs/ui-fix.md` (chart spec).
Referenced from `CLAUDE.md` -- read `docs/design.md` before any UI work, use `frontend-design` skill.

## Deliverables

1. `docs/design.md` -- design system spec (color tokens, component policy, provider palette)
2. `global.css` `@theme` block -- custom semantic tokens (provider, token type, decision, file action, chart)
3. Component refactors below
4. Delete `chart-colors.ts`

---

## Color System Cleanup

### Raw oklch values to remove

| File | Lines | What |
|------|-------|------|
| `chart-colors.ts` | all | Delete entire file -- replace with `@theme` tokens |
| `DetailPanel.svelte` | 47-58 | `:global()` JSON highlight colors -- move to global.css |
| `SettingsSection.svelte` | 180-191 | `:global()` JSON highlight colors -- duplicate of above |

### JSON syntax highlighting

Defined in TWO places (DetailPanel + SettingsSection) with identical raw oklch values. Consolidate into a single definition in `global.css`.

---

## DaisyUI Component Adoption

### `title=` -> `tooltip`

| File | Line | Current |
|------|------|---------|
| `Sidebar.svelte` | 30 | `title={item.label}` |
| `ThemeToggle.svelte` | 8 | `title="Toggle theme"` |
| `VmStateIndicator.svelte` | 7 | `title={collapsed ? vmStore.vmState : undefined}` |
| `SettingsSection.svelte` | 230 | `title=` on setting controls |

### Manual tabs -> `tabs` / `tab`

| File | Lines | Current |
|------|-------|---------|
| `ToolsTab.svelte` | 89-97 | Button group with conditional `btn-primary`/`btn-ghost` for Native/MCP sub-tabs |
| `StatsView.svelte` | via SubMenu | SubMenu acts as tab selector for AI/Tools/Network/Files |

### Manual expand/collapse -> `collapse`

| File | Lines | Current |
|------|-------|---------|
| `ModelsTab.svelte` | 138-153 | Manual toggle with SVG chevron rotation for trace expansion |
| `SettingsSection.svelte` | 441-445, 468-473 | Manual toggle with SVG chevron for settings groups |

### Custom nav lists -> `menu`

| File | Lines | Current |
|------|-------|---------|
| `SubMenu.svelte` | 22-51 | Custom button list with manual active state styling |
| `Sidebar.svelte` | nav section | Custom icon button column |

### Raw text stats -> `stat`

| File | Lines | Current |
|------|-------|---------|
| `StatsBar.svelte` | 14-20 | Tokens/tools/cost as plain text with pipe separators |

### Manual dividers -> `divider`

| File | Lines | Current |
|------|-------|---------|
| `SubMenu.svelte` | 25 | `border-t border-base-300` |
| `StatsBar.svelte` | 14 | `border-t border-base-300` |
| `StatusBar.svelte` | 6 | `border-t border-base-300` |
| `SettingsSection.svelte` | 201, 475 | `border-b border-base-300/50` |
| `ModelsTab.svelte` | 161 | `border-t border-base-200/50` |
| `NetworkTab.svelte` | 91, 154 | `border-b`/`border-t border-base-200` |
| `FilesTab.svelte` | 91, 143 | `border-b`/`border-t border-base-200` |
| `StatsBar.svelte` | 16, 18 | `text-base-content/20` pipe characters as separators |

### Wrapper divs -> `card`

| File | Lines | Current |
|------|-------|---------|
| `SettingsSection.svelte` | 199 | `rounded-md border border-base-300 bg-base-200/30` |
| `SettingsSection.svelte` | 414 | `rounded-lg border border-base-300` |
| `DetailPanel.svelte` | 61 | `border-l border-base-300` panel container |

---

## Skills & References

CLAUDE.md tells Claude to:
- Read `docs/design.md` before building or modifying any UI component
- Use the `frontend-design` skill for UI work

### What `docs/design.md` must define

- Two-layer color system: DaisyUI tokens for UI chrome, custom `@theme` tokens for domain semantics
- Provider palette: anthropic=orange, google=blue, openai=green, mistral=red (full Tailwind shade ranges)
- Token type palette: input, output, cache
- Decision palette: allowed, denied
- File action palette: created=blue, modified=sky, deleted=purple
- Chart infrastructure: grid, label
- DaisyUI component policy table (which component for which pattern)
- No raw color values in .svelte or .ts files
- No inline style= for colors
- Icons use currentColor inheritance
- Text hierarchy via base-content opacity variants
