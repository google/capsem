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
  // The vendor's scale already maps CSS coordinates to backing pixels,
  // including mouse input. Use it for density rather than stretching a
  // low-resolution canvas. Server capabilities replace this pre-hello bound.
  const displayLimits = new WeakMap();
  const viewportObservers = new WeakMap();
  function watchViewport(client) {
    if (typeof ResizeObserver === "undefined" || viewportObservers.has(client)) return;
    const parent = client.container?.parentElement;
    if (!parent) return;
    // Observing the parent avoids a feedback loop from our own scale styles.
    const observer = new ResizeObserver(() => client._screen_resized());
    observer.observe(parent);
    viewportObservers.set(client, observer);
  }
  function updateDensity(client) {
    const parent = client.container.parentElement;
    const width = parent.clientWidth;
    const height = parent.clientHeight;
    if (!(width > 0 && height > 0)) return false;
    const requested = globalThis.devicePixelRatio;
    const density = Number.isFinite(requested) && requested > 0 ? requested : 1;
    const [maxWidth, maxHeight] = displayLimits.get(client) || [8192, 4096];
    const scale = Math.min(density, maxWidth / width, maxHeight / height);
    if (scale === client.scale) return false;
    client.scale = scale;
    Object.assign(client.container.style, {
      width: `${100 * scale}%`, height: `${100 * scale}%`,
      transform: `scale(${1 / scale})`, transformOrigin: "top left",
    });
    for (const win of Object.values(client.id_to_window || {})) win.scale = scale;
    return true;
  }
  const init = XpraClient.prototype.init;
  XpraClient.prototype.init = function () {
    updateDensity(this);
    const result = init.apply(this, arguments);
    watchViewport(this);
    return result;
  };
  const close = XpraClient.prototype.close;
  XpraClient.prototype.close = function () {
    viewportObservers.get(this)?.disconnect();
    viewportObservers.delete(this);
    return close.apply(this, arguments);
  };
  const dpi = XpraClient.prototype._get_DPI;
  XpraClient.prototype._get_DPI = function () {
    return Math.round(dpi.call(this) * (this.scale || 1));
  };
  const displayCaps = XpraClient.prototype._get_display_caps;
  XpraClient.prototype._get_display_caps = function () {
    // The base hello captures display geometry before _make_hello refreshes
    // it. Keep both halves of that handshake in the same physical space.
    updateDensity(this);
    if (this.container.clientWidth > 0 && this.container.clientHeight > 0) {
      this.desktop_width = this.container.clientWidth;
      this.desktop_height = this.container.clientHeight;
    }
    return displayCaps.apply(this, arguments);
  };
  const resized = XpraClient.prototype._screen_resized;
  XpraClient.prototype._screen_resized = function () {
    const parent = this.container.parentElement;
    // A view switch can temporarily hide the iframe. Retain its last useful
    // dimensions instead of configuring a zero-sized desktop in the guest.
    if (!(parent.clientWidth > 0 && parent.clientHeight > 0)) return;
    if (updateDensity(this)) this.desktop_width = -1;
    return resized.apply(this, arguments);
  };
  const hello = XpraClient.prototype._process_hello;
  XpraClient.prototype._process_hello = function (packet) {
    const maximum = packet[1]?.max_desktop_size;
    if (Array.isArray(maximum) && maximum.length === 2 &&
        maximum.every(value => Number.isSafeInteger(value) && value > 0)) {
      displayLimits.set(this, maximum);
    }
    const changed = updateDensity(this);
    const result = hello.apply(this, arguments);
    if (changed) {
      this.desktop_width = -1;
      this._screen_resized();
    }
    return result;
  };

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
    watchViewport(this);
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
    // Xpra 6.5 uses the mean DPI for XSettings, independently of the x/y
    // display dimensions. The pinned HTML5 client only supplies the axes;
    // without the mean, GTK/Electron keep the startup font/device scale.
    if (this.capabilities.dpi) this.capabilities.dpi[""] = this._get_DPI();
  };

  globalThis.CapsemSurface = Object.freeze({ fitGeometry, fit });
})();
