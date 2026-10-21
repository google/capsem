// Keep Xpra's pinned protocol worker intact. A private initialization message
// supplies only this surface's scoped credential before the vendor worker runs.
"use strict";
self.addEventListener("message", function initialize(event) {
  if (event.data?.type !== "capsem-surface-init") return;
  self.removeEventListener("message", initialize);
  const credential = event.data.credential;
  const NativeWebSocket = self.WebSocket;
  self.WebSocket = class extends NativeWebSocket {
    constructor(url, protocols) {
      if (new URL(url).origin !== location.origin.replace(/^http/, "ws")) throw new Error("Surface origin mismatch");
      const offered = typeof protocols === "string" ? [protocols] : [...(protocols || [])];
      super(url, [...offered, credential]);
    }
  };
  // This owned wrapper lives beside Protocol.js, preserving vendor-relative imports.
  importScripts("Protocol.js");
});
