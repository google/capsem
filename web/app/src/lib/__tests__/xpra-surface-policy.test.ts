// capsem-surface.js is served by the gateway with the pinned xpra-html5
// client. It runs here against stand-ins for the client's two classes, with
// the same classic-script global scope a browser gives it.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { describe, expect, it } from 'vitest';

const POLICY = readFileSync(
  new URL('../../../../../crates/capsem-gateway/src/surface/static/capsem-surface.js', import.meta.url),
  'utf8',
);
const WORKER_POLICY = readFileSync(
  new URL('../../../../../crates/capsem-gateway/src/surface/static/capsem-surface-worker.js', import.meta.url),
  'utf8',
);

// The client's classes, reduced to what the policy touches.
const CLIENT = `
class XpraClient {
  constructor(width, height) { this.desktop_width = width; this.desktop_height = height; this.capabilities = {}; }
  _get_desktop_size() { return [this.desktop_width, this.desktop_height]; }
  _make_hello() { this.capabilities.system_tray = true; this.capabilities.windows = true; }
  query_keyboard_map() { this.queried = true; }
  do_init_keyboard() { this.initialized = true; }
}
class XpraWindow {
  constructor(client, geometry, extra = {}) {
    this.client = client;
    Object.assign(this, { leftoffset: 0, rightoffset: 0, topoffset: 0, bottomoffset: 0 }, geometry);
    this.metadata = extra.metadata || {};
    this.windowtype = extra.windowtype || ['NORMAL'];
    this.override_redirect = Boolean(extra.override_redirect);
    this.tray = Boolean(extra.tray);
    this.sent = [];
    this.css = 0;
    this.screen_resized();
  }
  screen_resized() {}
  move_resize(x, y, w, h) { Object.assign(this, { x, y, w, h }); }
  updateCSSGeometry() { this.css += 1; }
  geometry_cb(win) { this.sent.push({ x: win.x, y: win.y, w: win.w, h: win.h }); }
}
`;

function load(globals: Record<string, unknown> = {}) {
  const context = vm.createContext({ document: { querySelector: () => null }, ...globals });
  vm.runInContext(CLIENT, context);
  vm.runInContext(POLICY, context);
  return (source: string) => vm.runInContext(source, context);
}

describe('capsem-surface.js', () => {
  it('uses normal key events when the embed cannot query the host keyboard layout', () => {
    const run = load({document: {querySelector: () => null, featurePolicy: {allowsFeature: () => false}}});
    expect(run('const client = new XpraClient(800, 600); client.query_keyboard_map(); ({map: client.keyboard_map, initialized: client.initialized, queried: client.queried})'))
      .toEqual({map: {}, initialized: true, queried: undefined});
    const allowed = load({document: {querySelector: () => null, featurePolicy: {allowsFeature: () => true}}});
    expect(allowed('const client = new XpraClient(800, 600); client.query_keyboard_map(); client.queried')).toBe(true);
  });
  it('gives only the pinned transport worker a scoped credential and confines WebSockets', () => {
    const credential = `capsem-preview.${'a'.repeat(64)}`;
    let removed = false;
    class Worker {
      messages: unknown[] = [];
      constructor(public url: string | URL) {}
      postMessage(value: unknown) { this.messages.push(value); }
    }
    class WebSocket {
      constructor(public url: string, public protocols: string[]) {}
    }
    const run = load({
      URL, Worker, WebSocket,
      location: { href: 'http://preview.localhost:19444/_capsem/surface/index.html', origin: 'http://preview.localhost:19444' },
      document: { querySelector: () => ({ content: credential, remove: () => { removed = true; } }) },
    });
    const worker = run('new Worker("js/Protocol.js")') as Worker;
    expect(String(worker.url)).toBe('http://preview.localhost:19444/_capsem/surface/js/capsem-surface-worker.js');
    expect(worker.messages).toEqual([{ type: 'capsem-surface-init', credential }]);
    const decoder = run('new Worker("js/OffscreenDecodeWorker.js")') as Worker;
    expect(String(decoder.url)).toBe('js/OffscreenDecodeWorker.js');
    expect(decoder.messages).toEqual([]);
    const socket = run('new WebSocket("ws://preview.localhost:19444/", "binary")') as WebSocket;
    expect(socket.protocols).toEqual(['binary', credential]);
    expect(() => run('new WebSocket("ws://other.localhost:19444/", "binary")')).toThrow('Surface origin mismatch');
    expect(removed).toBe(true);
  });

  it('initializes the protocol worker before loading its unchanged vendor script', () => {
    const credential = `capsem-preview.${'b'.repeat(64)}`;
    let initialize: ((event: {data: unknown}) => void) | undefined;
    const imported: string[] = [];
    let removed = false;
    class WebSocket {
      constructor(public url: string, public protocols: string[]) {}
    }
    const self = {
      WebSocket,
      addEventListener: (_name: string, listener: typeof initialize) => { initialize = listener; },
      removeEventListener: () => { removed = true; },
    };
    const context = vm.createContext({ self, URL, location: {origin: 'http://preview.localhost:19444'}, importScripts: (script: string) => imported.push(script) });
    vm.runInContext(WORKER_POLICY, context);
    expect(imported).toEqual([]);
    initialize!({data: {type: 'unrelated'}});
    expect(imported).toEqual([]);
    initialize!({data: {type: 'capsem-surface-init', credential}});
    expect(removed).toBe(true);
    expect(imported).toEqual(['Protocol.js']);
    const socket = new self.WebSocket('ws://preview.localhost:19444/', ['binary']);
    expect(socket.protocols).toEqual(['binary', credential]);
    expect(() => new self.WebSocket('ws://other.localhost:19444/', ['binary'])).toThrow('Surface origin mismatch');
  });

  it('sizes the main window to the viewport and tells the server', () => {
    const run = load();
    const win = run('new XpraWindow(new XpraClient(1000, 700), { x: 2400, y: 900, w: 1200, h: 800 })');
    expect({ x: win.x, y: win.y, w: win.w, h: win.h }).toEqual({ x: 0, y: 0, w: 1000, h: 700 });
    expect(win.sent).toEqual([{ x: 0, y: 0, w: 1000, h: 700 }]);
  });

  it('keeps refitting when the server or the viewport moves it', () => {
    const run = load();
    const win = run('globalThis.win = new XpraWindow(new XpraClient(1000, 700), { x: 0, y: 0, w: 1000, h: 700 })');
    expect(win.sent).toEqual([]);
    run('win.move_resize(300, 200, 1200, 800)');
    expect({ x: win.x, y: win.y, w: win.w, h: win.h }).toEqual({ x: 0, y: 0, w: 1000, h: 700 });
    run('win.client.desktop_width = 1400; win.client.desktop_height = 900; win.screen_resized()');
    expect(win.sent.at(-1)).toEqual({ x: 0, y: 0, w: 1400, h: 900 });
  });

  it('keeps dialogs inside the viewport without growing them', () => {
    const run = load();
    const dialog = run(
      `new XpraWindow(new XpraClient(1000, 700), { x: 900, y: 650, w: 400, h: 300 },
        { windowtype: ['DIALOG'], metadata: { 'transient-for': 1 } })`,
    );
    expect({ x: dialog.x, y: dialog.y, w: dialog.w, h: dialog.h }).toEqual({ x: 600, y: 400, w: 400, h: 300 });
    const huge = run(
      `new XpraWindow(new XpraClient(1000, 700), { x: -50, y: -10, w: 3000, h: 2000 }, { windowtype: ['DIALOG'] })`,
    );
    expect({ x: huge.x, y: huge.y, w: huge.w, h: huge.h }).toEqual({ x: 0, y: 0, w: 1000, h: 700 });
  });

  it('leaves menus, tooltips and trays where the server puts them', () => {
    const run = load();
    for (const extra of ['{ override_redirect: true }', '{ tray: true }']) {
      const win = run(`new XpraWindow(new XpraClient(1000, 700), { x: 1500, y: 900, w: 200, h: 100 }, ${extra})`);
      expect({ x: win.x, y: win.y }).toEqual({ x: 1500, y: 900 });
      expect(win.sent).toEqual([]);
    }
  });

  it('does not advertise a tray, so the app shows one window', () => {
    const run = load();
    const capabilities = run('const c = new XpraClient(1000, 700); c._make_hello(); c.capabilities');
    expect(capabilities).toEqual({ system_tray: false, windows: true });
  });
});
