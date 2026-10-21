// xterm.js base options shared by all terminal instances.
// Hardening: links disabled, proposed APIs off, scrollback capped.

import type { ITerminalOptions } from '@xterm/xterm';

export const TERMINAL_OPTIONS: ITerminalOptions = {
  cursorBlink: true,
  cursorStyle: 'block',
  // The PTY owns line endings. Bare LF must preserve the cursor column for
  // applications that redraw with relative cursor movements.
  convertEol: false,
  fontFamily:
    'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace',
  fontSize: 14,
  lineHeight: 1.0,
  scrollback: 0,
  allowProposedApi: false,
  allowTransparency: false,
  disableStdin: false,

  // SECURITY: Override default link handler. OSC 8 hyperlinks render as
  // underlined text but clicking does nothing. A compromised VM could inject:
  //   \x1b]8;;https://evil.com\x1b\\Click here\x1b]8;;\x1b\\
  linkHandler: {
    activate: () => {
      // Intentionally empty -- links are NOT clickable in sandboxed terminals
    },
  },
};
