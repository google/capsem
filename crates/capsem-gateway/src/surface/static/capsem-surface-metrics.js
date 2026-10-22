// Opt-in display telemetry. No vendor changes, payload copies, input contents,
// remote logging, or continuous animation loop. Idle views report zero FPS.
"use strict";

class CapsemSurfaceMetrics {
  constructor(now) {
    this.started = now;
    this.frames = [];
    this.pending = {};
    this.inputTimes = {};
    this.inputSamples = {};
    this.codec = null;
    this.color = null;
    this.quality = null;
    this.decodeMs = null;
    this.decodeErrors = 0;
    this.paintMs = null;
    this.frameMs = null;
  }

  draw(packet) {
    const options = packet[10] || {};
    const text = value => typeof value === "string" ? value.slice(0, 32) :
      value instanceof Uint8Array ? new TextDecoder().decode(value.subarray(0, 32)) : null;
    this.codec = text(packet[6]);
    const color = options.subsampling || options.rgb_format || options.csc;
    this.color = text(color);
    this.quality = Number.isFinite(options.quality) ? options.quality : null;
  }

  ack(microseconds) {
    if (microseconds < 0) this.decodeErrors += 1;
    else if (Number.isFinite(microseconds)) this.decodeMs = microseconds / 1000;
  }

  input(kind, now) {
    // Keep the first outstanding input, not the last event of a rapid burst.
    if (this.pending[kind] == null) this.pending[kind] = now;
  }

  frame(now, paintMs) {
    const previous = this.frames.at(-1);
    this.frameMs = previous == null ? null : now - previous;
    this.frames.push(now);
    if (this.frames.length > 512) this.frames.shift();
    this.paintMs = paintMs;
    for (const kind of ["mouse", "key"]) {
      if (this.pending[kind] == null) continue;
      const elapsed = now - this.pending[kind];
      this.inputSamples[kind] = elapsed <= 1000 ? elapsed : null;
      this.inputTimes[kind] = now;
      delete this.pending[kind];
    }
  }

  sample(client, now) {
    this.frames = this.frames.filter(time => time > now - 1000);
    const active = this.frames.length > 0;
    const win = client.id_to_window?.[client.focused_wid] ||
      Object.values(client.id_to_window || {}).find(window => !window.tray && !window.override_redirect);
    const canvas = win?.canvas;
    const rect = canvas?.getBoundingClientRect();
    const numeric = value => Number.isFinite(value) && value >= 0 ? value : null;
    const input = kind => now - (this.inputTimes[kind] ?? -Infinity) <= 2000 ? this.inputSamples[kind] ?? null : null;
    return {
      // The offscreen worker does not expose actual presentation timestamps.
      // A damage acknowledgement is not evidence of a presented frame.
      fps: client.offscreen_api && client.decode_worker ? null :
        this.frames.length * 1000 / Math.max(100, Math.min(1000, now - this.started)),
      frameMs: active ? this.frameMs : null,
      paintMs: active ? this.paintMs : null,
      decodeMs: active ? this.decodeMs : null,
      decodeErrors: this.decodeErrors,
      rttMs: client.last_ping_echoed_time > 0 ? numeric(client.server_ping_latency) : null,
      mouseMs: input("mouse"), keyMs: input("key"),
      codec: this.codec, quality: this.quality, color: this.color,
      depth: numeric(win?.metadata?.depth ?? client.server_last_info?.display?.depth),
      clientDpi: numeric(client.capabilities?.dpi?.[""]),
      guestDpi: numeric(client.server_last_info?.display?.dpi?.value),
      sourceWidth: numeric(win?.w), sourceHeight: numeric(win?.h),
      backingWidth: numeric(canvas?.width), backingHeight: numeric(canvas?.height),
      displayWidth: numeric(rect?.width), displayHeight: numeric(rect?.height),
      dpr: numeric(globalThis.devicePixelRatio),
    };
  }
}

(function () {
  // Read only the trusted parent origin before capsem-surface.js removes the
  // bootstrap meta element. Diagnostics never read or forward its credential.
  const parentOrigin = document.querySelector('meta[name="capsem-surface-protocol"]')?.dataset?.parentOrigin;
  if (!parentOrigin) return;
  let client;
  let meter;
  let timer;
  let lastDisplayRequest = -Infinity;
  function disable() {
    if (timer != null) clearInterval(timer);
    timer = undefined;
    meter = undefined;
    lastDisplayRequest = -Infinity;
  }
  function publish() {
    if (meter && client && !document.hidden) {
      const now = performance.now();
      if (client.connected && now - lastDisplayRequest >= 2000) {
        // Ask only for public display diagnostics, never process/environment
        // info. The vendor owns response decoding and the current info value.
        // Xpra names these subsystems after the actual manager classes.
        client.send([PACKET_TYPES.info_request, [client.uuid], [], ["displaymanager", "x11displaymanager"]]);
        lastDisplayRequest = now;
      }
      parent.postMessage({type: "capsem:surface-metrics", metrics: meter.sample(client, now)}, parentOrigin);
    }
  }
  addEventListener("message", event => {
    if (event.source !== parent || event.origin !== parentOrigin ||
        event.data?.type !== "capsem:surface-debug" || typeof event.data.enabled !== "boolean") return;
    disable();
    if (event.data.enabled) {
      meter = new CapsemSurfaceMetrics(performance.now());
      timer = setInterval(publish, 500);
      publish();
    }
  });
  addEventListener("pagehide", disable);
  for (const [event, kind] of [["mousemove", "mouse"], ["mousedown", "mouse"], ["keydown", "key"]]) {
    document.addEventListener(event, () => meter?.input(kind, performance.now()), {capture: true, passive: true});
  }

  const open = XpraClient.prototype._process_open;
  XpraClient.prototype._process_open = function () {
    client = this;
    const result = open.apply(this, arguments);
    parent.postMessage({type: "capsem:surface-debug-ready"}, parentOrigin);
    return result;
  };
  const close = XpraClient.prototype._process_close;
  XpraClient.prototype._process_close = function () {
    disable();
    return close.apply(this, arguments);
  };
  const draw = XpraClient.prototype._process_draw;
  XpraClient.prototype._process_draw = function (packet) {
    meter?.draw(packet);
    return draw.apply(this, arguments);
  };
  const ack = XpraClient.prototype.do_send_damage_sequence;
  XpraClient.prototype.do_send_damage_sequence = function (_sequence, _window, _width, _height, decodeTime) {
    meter?.ack(decodeTime);
    return ack.apply(this, arguments);
  };
  const present = XpraClient.prototype.draw_pending_list;
  XpraClient.prototype.draw_pending_list = function () {
    const count = meter ? this.pending_redraw.length : 0;
    const start = count ? performance.now() : 0;
    const result = present.apply(this, arguments);
    if (count) {
      const now = performance.now();
      meter?.frame(now, now - start);
    }
    return result;
  };
})();
