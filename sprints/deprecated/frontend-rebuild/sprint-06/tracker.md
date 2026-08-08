# Sprint 06: Polish / Keyboard Shortcuts / Accessibility

UX quality pass — shortcuts, animations, responsive layout, accessibility.

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
Depends on: Sprint 05

## Acceptance Criteria

### Keyboard Shortcuts
- [ ] Cmd+T — new tab
- [ ] Cmd+W — close active tab
- [ ] Cmd+1 through Cmd+9 — switch to tab by position
- [ ] Cmd+Shift+[ — previous tab
- [ ] Cmd+Shift+] — next tab
- [ ] Shortcuts don't fire when terminal has focus (terminal captures its own keys)

### Tab Overflow
- [ ] Horizontal scroll when >8 tabs
- [ ] Scroll buttons (left/right arrows) appear at edges
- [ ] Active tab auto-scrolls into view

### Animations
- [ ] Tab slide-in on create (150ms ease-out)
- [ ] Tab fade-out on close (100ms ease-in)
- [ ] View switch transitions (cross-fade or slide)
- [ ] No animation when `prefers-reduced-motion` is set

### Responsive
- [ ] Layout adapts below 768px width
- [ ] Tab bar compresses (shorter labels, smaller padding)
- [ ] Toolbar collapses to essential actions

### Accessibility
- [ ] Focus rings on all interactive elements (buttons, tabs, inputs, links)
- [ ] Full keyboard navigation (Tab/Shift+Tab through UI, Enter to activate)
- [ ] ARIA labels on tabs (`role="tab"`, `aria-selected`, `aria-controls`)
- [ ] ARIA labels on buttons, form controls
- [ ] Skip-to-content link
- [ ] Screen reader announces tab switches

### Performance
- [ ] 60fps tab switching with 10 tabs open
- [ ] No layout thrashing during animations
- [ ] Idle iframes don't consume CPU

## Testing Gate

- [ ] All keyboard shortcuts work (manual verification)
- [ ] Tab overflow scrolls smoothly with 12+ tabs
- [ ] Lighthouse accessibility score >90
- [ ] Chrome DevTools MCP screenshot at 768px and 1440px widths
- [ ] `prefers-reduced-motion` disables animations
- [ ] 10 tabs open without visible jank
- [ ] `pnpm run check` passes
