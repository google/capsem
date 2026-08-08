# Sprint 03: Complex Tab Views (Files, Inspector)

Build views that need split panes and specialized editors.

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
Depends on: Sprint 01
Status: Done

## Acceptance Criteria

### Files View
- [x] `FilesView.svelte` -- split pane layout (tree left, content right)
- [x] `FileTree.svelte` -- collapsible directory tree with indent levels, self-import recursion
- [x] `FileContent.svelte` -- Shiki syntax highlighting with line numbers, breadcrumb nav, file size
- [x] Shiki themes mapped 1:1 to terminal theme families (Dracula, Nord, Catppuccin, etc.)
- [x] Theme reactivity: syntax colors update when user changes terminal theme or light/dark mode
- [x] Click file in tree to display content
- [x] Expand/collapse directories with click or arrow keys
- [x] File type icons (Phosphor: File, Folder, FolderOpen)
- [x] ARIA: `role="tree"`, `role="treeitem"`, `aria-selected`, `aria-expanded`

### Inspector View
- [x] `InspectorView.svelte` -- SQL editor + results
- [x] Monospace textarea for SQL input
- [x] Run button (Cmd+Enter)
- [x] Results table with sortable columns (click header to sort asc/desc)
- [x] Preset query dropdown (5 presets: Recent events, HTTP requests, Tool calls, Model calls, File events)
- [x] SQL injection defense: `validateSelectOnly()` refuses non-SELECT queries, blocks INSERT/UPDATE/DELETE/DROP/ALTER/CREATE/TRUNCATE/REPLACE/ATTACH/DETACH/PRAGMA
- [x] Error display for failed/rejected queries

### Mock Data
- [x] `mock.ts` -- file tree (4 dirs, 7 files with Rust/TOML/Markdown content)
- [x] `mock.ts` -- 5 preset SQL queries with tabular result sets
- [x] `findFileNode()` recursive path lookup helper
- [x] `validateSelectOnly()` and `executeMockQuery()` pure functions

### Toolbar / Routing
- [x] Files + Inspector buttons in view switcher (Toolbar.svelte, Phosphor: FolderSimple, MagnifyingGlassPlus)
- [x] App.svelte routes files/inspector views with terminal preserved via hidden class

### Dependencies Added
- [x] `shiki` -- syntax highlighting (themes match terminal theme families)

## Testing Gate

- [x] `pnpm run check` passes (0 errors, 3 pre-existing warnings)
- [x] `pnpm run build` passes
- [x] 185 vitest tests pass (162 existing + 23 new)
- [x] SQL validation: 10 test cases (valid SELECT, empty, comments, non-SELECT, dangerous keywords, case insensitive)
- [x] File tree mock data: 5 tests (structure, unique paths, parent-child paths, findFileNode)
- [x] Inspector mock data: 4 tests (presets have results, executeMockQuery matches)
- [x] Chrome DevTools MCP screenshot of each view in light + dark
- [x] No console errors (2 pre-existing iframe sandbox warnings from terminal)

### Screenshots
- `screenshots/sprint-03-files-dark.png` -- Files view, dark mode, syntax highlighting
- `screenshots/sprint-03-files-light.png` -- Files view, light mode, syntax highlighting
- `screenshots/sprint-03-inspector-dark.png` -- Inspector view, dark mode, HTTP requests preset
- `screenshots/sprint-03-inspector-light.png` -- Inspector view, light mode, Recent events preset
