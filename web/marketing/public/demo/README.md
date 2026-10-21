# Hero demo assets

Dock icons are downloaded PNGs from the user-selected macOSicons library:

- `chrome.png`: [Chrome by iuriveiga](https://macosicons.com/icon/chrome-iHEQxMfnet).
- `terminal.png`: [Terminal by robertgs](https://macosicons.com/icon/terminal-eJCDM3QFYI).
- `capsem.png`: canonical `web/graphics/tauri/128x128.png` from this repository.

The small session action/menu SVGs are unmodified [Phosphor regular icons](https://github.com/phosphor-icons/core/tree/main/assets/regular), matching the product's icon library. License: `PHOSPHOR-LICENSE.txt`.

`capsem-ui.woff2` is a Latin subset of the product's
`web/app/public/fonts/GoogleSansFlex-Variable.ttf`, with weight 400–700 and the
other axes fixed to their defaults. License: `FONT-OFL.txt`.

The Scale scene is a passive excerpt of the Sessions table in
`web/app/src/lib/components/shell/NewTabPage.svelte`, with its TabBar and
Toolbar. Session names and usage figures are fictional demo data. Marketing
labels sit outside the product window. Compact widths show the first three
columns; larger widths also show uptime, tokens, cost and actions.

The continuous TUI scenes follow `crates/capsem-tui/src/ui.rs`: its exact
palette, bottom status/session bar, `sessions` overlay, `new session` dialog,
name input, profile list and Alt+N/Enter shortcuts. The checked-in CLI opens
the TUI with `capsem shell`. The new Email productivity session selects the
Co-work profile, then launches Codex inside that VM; the Website session stays
alive. Dock/window/cursor actions are simulated animation, with pointer targets
measured from the icon, table row and xterm URL positions.
