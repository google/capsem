// Reconnecting an embedded viewer needs a fresh scoped session. Reusing its
// old WebSocket credential after idle/sleep retries an expired preview forever.
"use strict";

(function () {
  const origin = document.querySelector('meta[name="capsem-surface-protocol"]')?.dataset?.parentOrigin;
  if (!origin) return;
  const established = new WeakSet();
  const complete = XpraClient.prototype._process_startup_complete;
  XpraClient.prototype._process_startup_complete = function () {
    const result = complete.apply(this, arguments);
    established.add(this);
    return result;
  };
  const reconnect = XpraClient.prototype.do_reconnect;
  XpraClient.prototype.do_reconnect = function () {
    // Initial startup retains the existing finite deadline. A server's
    // explicit disconnect also retains its normal final-close behavior.
    if (!established.has(this)) return reconnect.apply(this, arguments);
    if (this.closed) return;
    this.close(); // Vendor cleanup cancels pending timers, workers and audio.
    parent.postMessage({type: "capsem:surface-reconnect"}, origin);
  };
})();
