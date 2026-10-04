// Capsem's policy for the pinned xpra-html5 client of an Xpra surface: one
// application, fitted to the browser viewport.
//
// The gateway loads this after the client's own scripts and before the client
// starts, so it adjusts the client's classes and the vendored files stay
// byte-identical to the pinned package.
//
// Xpra keeps a window's previous X11 position, and the v0.7 spike saw Claude
// Desktop's 1200x800 window open beyond a narrower browser viewport. So the
// application's main window is sized to the viewport and every other window
// (dialogs, transients) is kept inside it, both when a window appears and
// whenever the server or the viewport moves it. The new geometry is sent to
// the server, so the application lays itself out for the space it really has.
"use strict";

(function () {
  /** The geometry a window gets inside a viewport of `width` x `height`. */
  function fitGeometry(win, width, height, main) {
    const left = win.leftoffset || 0;
    const top = win.topoffset || 0;
    const maxWidth = Math.max(1, width - left - (win.rightoffset || 0));
    const maxHeight = Math.max(1, height - top - (win.bottomoffset || 0));
    if (main) {
      return { x: left, y: top, w: maxWidth, h: maxHeight };
    }
    const w = Math.min(win.w, maxWidth);
    const h = Math.min(win.h, maxHeight);
    return {
      x: Math.min(Math.max(win.x, left), left + maxWidth - w),
      y: Math.min(Math.max(win.y, top), top + maxHeight - h),
      w,
      h,
    };
  }

  /** Menus, tooltips and the like place themselves; trays are not shown. */
  function isApplicationWindow(win) {
    return !win.override_redirect && !win.tray && !win.client.server_is_desktop && !win.client.server_is_shadow;
  }

  /** The application's own top-level window, as opposed to its dialogs. */
  function isMainWindow(win) {
    const metadata = win.metadata || {};
    if (metadata["transient-for"] || metadata.modal) {
      return false;
    }
    const types = win.windowtype || [];
    return types.length === 0 || types.includes("NORMAL");
  }

  /** Fit `win` to the viewport; true when its geometry changed. */
  function fit(win) {
    if (!isApplicationWindow(win)) {
      return false;
    }
    const [width, height] = win.client._get_desktop_size();
    if (!(width > 0 && height > 0)) {
      return false;
    }
    const next = fitGeometry(win, width, height, isMainWindow(win));
    if (next.x === win.x && next.y === win.y && next.w === win.w && next.h === win.h) {
      return false;
    }
    Object.assign(win, next);
    win.updateCSSGeometry();
    return true;
  }

  const screenResized = XpraWindow.prototype.screen_resized;
  XpraWindow.prototype.screen_resized = function () {
    const changed = fit(this);
    screenResized.call(this);
    if (changed) {
      this.geometry_cb(this);
    }
  };

  const moveResize = XpraWindow.prototype.move_resize;
  XpraWindow.prototype.move_resize = function (x, y, w, h) {
    moveResize.call(this, x, y, w, h);
    if (fit(this)) {
      this.geometry_cb(this);
    }
  };

  // Advertising a system tray made Claude Desktop show a second, 32x32
  // surface beside its window. The application keeps its window instead.
  const makeHello = XpraClient.prototype._make_hello;
  XpraClient.prototype._make_hello = function () {
    makeHello.call(this);
    this.capabilities.system_tray = false;
  };

  globalThis.CapsemSurface = Object.freeze({ fitGeometry, fit });
})();
