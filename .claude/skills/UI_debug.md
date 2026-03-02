# UI Debug & Visual Verification Skill

Use this skill for systematic visual verification of the Capsem frontend using Chrome DevTools MCP tools.

## Prerequisites

- `just ui` running (Astro dev server on http://localhost:5173)
- Chrome browser open (DevTools MCP connected)
- Verify dev server is up: `mcp__chrome-devtools__list_pages` should show a page

## Quick Health Check

Before any visual work, run a fast health check:

```
1. navigate_page to http://localhost:5173
2. list_console_messages types=["error","warn"] -- expect zero
3. take_screenshot fullPage=true -- verify page renders at all
```

If the page is blank or has errors, stop and fix the build first (`cd frontend && pnpm run check`).

## Full Visual Verification Workflow

### Phase 1: Page Load & Console

1. **Navigate**: `navigate_page` to `http://localhost:5173`
2. **Wait**: `wait_for` text `["Capsem"]` or a known UI element
3. **Console check**: `list_console_messages` with `types: ["error", "warn"]`
   - Zero errors expected (filter out known framework noise like Vite HMR)
   - Warnings about missing env vars in mock mode are OK
4. **Full-page screenshot**: `take_screenshot` with `fullPage: true`

### Phase 2: View Navigation

Walk every main view via the sidebar:

1. **Get snapshot**: `take_snapshot` to get element UIDs
2. **Click sidebar items**: For each view (Terminal, Analytics, Settings):
   a. `click` the sidebar button UID
   b. `take_screenshot` with `fullPage: true`
   c. `list_console_messages` types=["error"] -- verify no new errors
3. **Wizard view**: On first load with mock data (no API keys configured), the wizard should appear automatically. Verify:
   a. "Welcome to Capsem" heading is visible
   b. "Configure Providers" button exists and works (click it, verify Settings view opens to AI Providers)
   c. "Skip for now" button exists and works (click it, verify Terminal view opens)

### Phase 3: Settings View Deep Dive

The settings view auto-generates its left nav from the TOML tree. Verify each section:

1. **Left nav**: `take_snapshot` to see all nav items. Expected sections (from defaults.toml):
   - AI Providers
   - Package Registries
   - Search
   - Guest Environment
   - Network
   - VM
   - Appearance

2. **Click through every section**, screenshot each:

   **AI Providers section**:
   - Provider cards for Anthropic, OpenAI, Google AI (rounded border cards)
   - Each card has a toggle in the header (Allow Anthropic, Allow OpenAI, Allow Google AI)
   - Google AI toggle should be ON by default, others OFF
   - Disabled provider cards should have dimmed content (opacity)
   - API key inputs with eye/reveal toggle button
   - Domain text inputs (*.anthropic.com, etc.)
   - Sub-groups: "Claude Code", "Gemini CLI" with nested file-type settings
   - Collapsed/advanced settings behind "Show N advanced settings" expander
   - Click the expander to verify collapsed settings (settings.json, state.json) appear

   **Package Registries section**:
   - Toggle cards for GitHub, npm, PyPI, crates.io
   - All ON by default
   - Domain metadata shown (github.com, registry.npmjs.org, etc.)

   **Search section**:
   - Toggle cards for Google Search, Perplexity, Firecrawl
   - Google Search ON by default, others OFF
   - Domain metadata shown

   **Guest Environment section**:
   - Sub-groups: Shell, TLS
   - Text inputs for TERM, HOME, PATH, LANG
   - CA bundle path

   **Network section**:
   - Default action dropdown (allow/deny choices)
   - Custom allowed domains text input
   - Custom blocked domains text input

   **VM section**:
   - Boolean toggle: Log request bodies
   - Number inputs with range hints: Max body capture (0--1048576), Session retention (1--365),
     CPU cores (1--8), RAM (1--16), Scratch disk size (1--128), Maximum sessions (1--10000),
     Maximum disk usage (1--1000), Terminated session retention (30--3650)

   **Appearance section**:
   - Dark mode toggle (verify it works live -- click it, page theme should change)
   - Font size number input (8--32)

### Phase 4: Interaction Testing

1. **API key reveal**: Find an API key input, click the eye button next to it
   - `take_snapshot` before and after to verify input type changes (password -> text)

2. **Provider toggle**: Toggle a provider ON/OFF
   - When OFF: child settings should be dimmed/disabled (opacity, pointer-events)
   - When ON: child settings should be interactive
   - `take_screenshot` in both states

3. **Advanced settings expander**: Find a group with collapsed settings
   - Click "Show N advanced settings"
   - Verify the collapsed settings appear
   - Click again to hide them
   - `take_screenshot` in both states

4. **Theme toggle**: In Appearance section
   - Click the dark mode toggle
   - `take_screenshot` -- entire UI should switch theme (light <-> dark)
   - Click again to switch back
   - `take_screenshot` -- verify original theme restored

5. **Number inputs**: Check that number inputs have proper min/max attributes
   - `take_snapshot verbose=true` to see input attributes

### Phase 5: Lint Issue Display

If any lint issues exist (mock data includes a warning for Google AI empty API key):

1. Navigate to AI Providers section
2. Look for inline warning/error text below the relevant input
3. `take_screenshot` to capture the lint badge
4. Verify the message text is human-readable

### Phase 6: Theme Verification (Both Modes)

1. **Dark mode** (default):
   - `take_screenshot` of Settings view
   - Verify dark background, light text, proper contrast on cards/badges

2. **Light mode**:
   - Toggle theme via Appearance > Dark mode
   - `take_screenshot` of Settings view
   - Verify light background, dark text, proper contrast
   - Check that toggle states, badges, and borders are visible in both themes

3. **Color rules** (FIRM -- see frontend.md skill):
   - Blue = positive (info badge for modified, primary for active states)
   - Purple = negative (secondary badge for corp-locked)
   - No green or red in UI chrome

### Phase 7: Responsive / Edge Cases

1. **Empty sections**: Sections with no settings should show gracefully (not crash)
2. **Long values**: API key inputs, file textareas, domain lists should not overflow
3. **Corp-locked badge**: Settings with corp_locked should show a "corp" badge
4. **Source badge**: User-modified settings should show a "modified" badge
5. **Resize**: `resize_page` to a narrow width (800x600) and verify layout doesn't break

## Pass Criteria

- [ ] Zero console errors (warnings OK if framework-related)
- [ ] All 7+ settings sections render from the tree (no hardcoded sections)
- [ ] All control types work: bool toggle, apikey+reveal, number+range, text, select, file textarea
- [ ] Provider toggle enables/disables child settings visually
- [ ] Advanced settings expand/collapse
- [ ] Lint issues display inline below the relevant control
- [ ] Theme toggle works live
- [ ] Both light and dark themes render correctly
- [ ] Wizard view appears on first load (no API keys)
- [ ] Wizard "Configure Providers" navigates to settings
- [ ] Wizard "Skip" navigates to terminal
- [ ] No hardcoded section names or icons in the settings left nav

## Common Issues

### "Settings view is empty"

- Check that `settingsStore.load()` was called in `App.svelte` `onMount`
- Check console for errors from `getSettingsTree()` or `lintConfig()`
- In mock mode, verify `MOCK_SETTINGS_TREE` in `mock.ts` is well-formed

### "Section nav doesn't highlight"

- The `active` prop on SubMenu must match the section name exactly (derived from tree group names)
- Check that `sidebarStore.settingsSection` is set to a valid section name

### "Theme toggle doesn't work"

- The SettingsSection component must special-case `appearance.dark_mode` to call `themeStore.toggle()`
- Verify `themeStore` is imported and accessible

### "Collapsed settings not showing"

- Check that the `collapsed` field on settings is set in defaults.toml
- The `partitionChildren` function in SettingsSection.svelte must separate collapsed/non-collapsed
- The "Show N advanced settings" button must toggle the `showAdvanced` state
