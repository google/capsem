# Owned graphical client repairs

The upstream package remains pinned by URL and SHA-256 in `SOURCE.toml`.
Declared `patches/` inputs are applied only after that package verifies.
`SHA256SUMS` covers the pin, license, this note, patch and generated client.
The gateway build verifies the complete inventory and embeds only `www/`.

The patch changes client resource ownership:

- Input handlers are installed once for the client, rather than once per
  connection-state reset. Repeated resets previously multiplied mouse and
  wheel processing.
- The bell audio context lives for the client and closes on final shutdown.
  Reconnects previously allocated another context without closing the old one.
- A reconnect timer has an owned handle. Final close cancels it, rejects late
  retries and is idempotent. Previously a delayed retry could access a drawing
  DOM already replaced by Capsem's disconnected control.
- Final disconnect invokes the close callback once.

`patches/canvas-mouse-propagation.patch` retains the delegated mouse callbacks'
return values. Xpra's window callbacks return false when they consume an event;
jQuery then stops propagation. Discarding that result let the root screen
listener send a second pointer packet for the same browser event. Mouse events
outside a window still reach the screen listener, and callbacks that return
true retain their propagation behavior.

`patches/notification-text.patch` creates notification titles, bodies and
action labels with text APIs. Guest-controlled strings cannot create markup
or attributes. Notification IDs use attribute assignment and exact DOM lookup,
never HTML interpolation or CSS selector construction; icon lookup stays
within the notification. Action identity and close callbacks are retained.

Normal reconnect still resets connection state and retains the original
upstream transport, parser, geometry and window behavior. No codec, color,
security policy or guest application setting changes here.

Regenerate from the repository root through the bounded wrapper:

```text
uv run --project build_system --frozen python build_system/scripts/build/vendor_xpra_html5.py
```

Do not edit generated `www/` bytes or update checksums to accept an unexplained
edit. A changed package must pass its own hash check and the declared patch
must apply. Upstream reference:
https://github.com/Xpra-org/xpra-html5/blob/master/html5/js/Client.js

`web/app/src/lib/__tests__/xpra-client-lifecycle.test.ts` exercises the actual
generated client with browser/platform boundaries isolated: final-close retry,
late events, duplicate scheduling, single callback, repeated reconnect, input
handler counts and audio lifetime. Real graphical acceptance separately proves
the browser, transport and guest input path.
