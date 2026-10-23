# Hero demo assets

Dock icons are downloaded PNGs from the user-selected macOSicons library:

- `chrome.png`: [Chrome by iuriveiga](https://macosicons.com/icon/chrome-iHEQxMfnet).
- `terminal.png`: [Terminal by robertgs](https://macosicons.com/icon/terminal-eJCDM3QFYI).
- `capsem.png`: canonical `web/graphics/tauri/128x128.png` from this repository.

The small session action/menu SVGs are unmodified [Phosphor regular icons](https://github.com/phosphor-icons/core/tree/main/assets/regular), matching the product's icon library. License: `PHOSPHOR-LICENSE.txt`.

`capsem-ui.woff2` is a Latin subset of the product's
`web/app/public/fonts/GoogleSansFlex-Variable.ttf`, with weight 400–700 and the
other axes fixed to their defaults. License: `FONT-OFL.txt`.

The Scale scene uses passive excerpts of the Sessions table in
`web/app/src/lib/components/shell/NewTabPage.svelte`, with its TabBar and
Toolbar, plus the profile launcher and `CreateSandboxDialog.svelte` form.
Only created sessions are listed: the four additional VMs are named, confirmed
and booted before their tabs receive prompts. Every session has a separate
launch timestamp, transcript and usage progression; inactive tabs keep working.
The scene stays in the GUI. Guest agents never inspect other VMs.
Session names, transcripts and usage figures are fictional demo data. Marketing
labels sit outside the product window. Compact widths show the first three
columns; larger widths also show uptime, tokens, cost and actions.

The continuous TUI scenes follow `crates/capsem-tui/src/ui.rs`: its
palette, bottom status/session bar, `sessions` overlay, `new session` dialog,
name input, profile list and Alt+N/Enter shortcuts. The checked-in CLI opens
the TUI with `capsem shell`. The new Email productivity session selects the
Co-work profile, then launches Codex inside that VM; the Website session stays
alive. Creation now holds on the confirmation and the new running session row.

AGY's ANSI logo and header have a single source, `HERO_AGY_BANNER`, rendered
through xterm in both the TUI and GUI. Codex's box and composer also share one
component.

Dock motion follows the distance-based scale, lateral nudge and spring model in
[Build UI's Magnified Dock](https://buildui.com/recipes/magnified-dock).
The small Svelte implementation uses fixed icon anchors, a 60fps shared clock,
substepped springs and a separate launch bounce. Pointer targets never read
transformed icon bounds, so magnification cannot pull the pointer around.
Pausing the player freezes the pointer, springs, bounce and agent progress.

Focused regressions are in `web/marketing/tests/hero-demo.test.mjs` (Node's
built-in test runner, with TypeScript stripping). They cover isolation, explicit
session creation, continuity through the dock's center and spring stability.
