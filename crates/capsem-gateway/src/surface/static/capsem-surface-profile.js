// Negotiation runs after decoder validation, before the final hello is sent.
// Xpra 6.5.1 consumes encoding.rgb_lz4 and top-level lz4/brotli; both pixel
// compression and packet compression must be disabled for the local path.
"use strict";
(function () {
  const local = document.querySelector('meta[name="capsem-surface-protocol"]')?.dataset.displayProfile === "local";
  const permitted = new Set(local
    ? ["rgb", "rgb24", "rgb32", "scroll", "void"]
    : ["rgb", "rgb24", "rgb32", "jpeg", "webp", "png", "png/P", "png/L", "scroll", "void"]);
  const hello = XpraClient.prototype._make_hello;
  XpraClient.prototype._make_hello = function () {
    hello.call(this);
    const caps = this.capabilities;
    const encodings = this.supported_encodings.filter(encoding => permitted.has(encoding));
    caps.encodings[""] = encodings;
    caps.encodings.core = encodings;
    caps.encoding[""] = local ? "rgb" : "auto";
    caps.encoding.video_scaling = false;
    caps.encoding.full_csc_modes = local ? {} : {
      webp: ["BGRX", "BGRA"],
      jpeg: ["BGRX", "BGRA", "BGR", "RGBX", "RGBA", "RGB"],
    };
    if (local) {
      caps.encoding.rgb_lz4 = false;
      caps.compression_level = 0;
      caps.lz4 = false;
      caps.brotli = false;
    }
  };
})();
