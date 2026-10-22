// A guest PNG is in desktop pixels; a CSS cursor is outside the desktop's
// transform. Declare its density instead of decoding/resampling another bitmap.
// https://www.w3.org/TR/css-ui-4/#cursor
"use strict";
(function () {
  XpraWindow.prototype.update_cursor_density = function () {
    if (!this.cursor_data) return;
    const [url, x, y] = this.cursor_data;
    const requested = this.client.scale;
    const scale = Number.isFinite(requested) && requested > 0 ? requested : 1;
    jQuery(`#${this.wid}`).css("cursor", `image-set(url("${url}") ${scale}x) ${x / scale} ${y / scale}, auto`);
  };
  XpraWindow.prototype.set_cursor = function (encoding, w, h, xhot, yhot, data) {
    if (encoding !== "png") {
      this.warn("received an invalid cursor encoding:", encoding);
      return;
    }
    // Keep physical geometry for Xpra's remote-pointer overlay, which lives
    // inside the transformed desktop. The CSS cursor alone uses logical units.
    this.cursor_data = [this.construct_base64_image_url(encoding, data), xhot, yhot, w, h];
    this.update_cursor_density();
  };
})();
