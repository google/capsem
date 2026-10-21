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
  // Embedded documents receive a scoped session in trusted HTML, independently
  // of third-party cookies. Only the pinned transport worker gets it, and the
  // confined router removes it before the workload sees the WebSocket offer.
  const session = document.querySelector('meta[name="capsem-surface-protocol"]');
  if (session) {
    const credential = session.content;
    session.remove();
    const NativeWorker = globalThis.Worker;
    const protocolURL = new URL("js/Protocol.js", location.href).href;
    globalThis.Worker = class extends NativeWorker {
      constructor(url, options) {
        const protocol = new URL(url, location.href).href === protocolURL;
        super(protocol ? new URL("js/capsem-surface-worker.js", location.href) : url, options);
        if (protocol) this.postMessage({ type: "capsem-surface-init", credential });
      }
    };
    const NativeWebSocket = globalThis.WebSocket;
    globalThis.WebSocket = class extends NativeWebSocket {
      constructor(url, protocols) {
        if (new URL(url).origin !== location.origin.replace(/^http/, "ws")) throw new Error("Surface origin mismatch");
        const offered = typeof protocols === "string" ? [protocols] : [...(protocols || [])];
        super(url, [...offered, credential]);
      }
    };
  }

  // Embedded contexts may not query the host's keyboard layout. Use Xpra's
  // existing key-event path without requesting additional browser authority.
  const queryKeyboardMap = XpraClient.prototype.query_keyboard_map;
  XpraClient.prototype.query_keyboard_map = function () {
    const policy = document.permissionsPolicy || document.featurePolicy;
    if (policy && !policy.allowsFeature("keyboard-map")) {
      this.keyboard_map = {};
      this.do_init_keyboard();
      return;
    }
    queryKeyboardMap.call(this);
  };

  // A running container may still be starting Xpra's listener. Keep the
  // initial same-origin socket attempt here, with one finite deadline; the
  // vendor retains ownership after the first successful WebSocket open.
  const startup = new WeakMap();
  const connect = XpraClient.prototype.connect;
  XpraClient.prototype.connect = function () {
    const previous = startup.get(this);
    if (previous && !previous.opened && (previous.ended || Date.now() >= previous.deadline)) {
      previous.ended = true;
      // A vendor reconnect timer may already be firing. End it before it
      // can create another socket beyond our initial startup deadline.
      this.reconnect_in_progress = false;
      this.close();
      return;
    }
    if (!startup.has(this)) {
      startup.set(this, { deadline: Date.now() + 60_000, opened: false });
      const parentOrigin = session?.dataset?.parentOrigin;
      if (parentOrigin) {
        this.callback_close = () => {
          const main = document.createElement("main");
          main.className = "capsem-surface-disconnected";
          const title = document.createElement("h1");
          title.textContent = "App disconnected";
          const retry = document.createElement("button");
          retry.type = "button";
          retry.textContent = "Reconnect";
          retry.onclick = () => parent.postMessage({ type: "capsem:surface-reconnect" }, parentOrigin);
          main.append(title, retry);
          document.body.replaceChildren(main);
        };
      }
    }
    return connect.apply(this, arguments);
  };
  const processOpen = XpraClient.prototype._process_open;
  XpraClient.prototype._process_open = function () {
    const state = startup.get(this);
    if (state) state.opened = true;
    return processOpen.apply(this, arguments);
  };
  const processError = XpraClient.prototype._process_error;
  XpraClient.prototype._process_error = function (packet) {
    const state = startup.get(this);
    if (state && !state.opened && Date.now() < state.deadline && [0, 1006].includes(Number(packet[2]))) {
      if (!this.reconnect_in_progress) this.do_reconnect();
      return;
    }
    if (state && !state.opened) state.ended = true;
    return processError.call(this, packet);
  };
  const processClose = XpraClient.prototype._process_close;
  XpraClient.prototype._process_close = function (packet) {
    const state = startup.get(this);
    if (state && !state.opened && (state.ended || Date.now() >= state.deadline)) {
      this.close();
      return;
    }
    return processClose.call(this, packet);
  };

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
