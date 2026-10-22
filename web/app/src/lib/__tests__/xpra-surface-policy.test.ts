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
  send_configure_window(win, state, skip_geometry) { this.configuration = {state, skip_geometry}; }
  query_keyboard_map() { this.queried = true; }
  do_init_keyboard() { this.initialized = true; }
  connect() { this.connects = (this.connects || 0) + 1; }
  _process_open() { this.opened = true; }
  _process_error() { this.errors = (this.errors || 0) + 1; }
  _process_close() { this.closes = (this.closes || 0) + 1; }
  do_reconnect() { this.retries = (this.retries || 0) + 1; this.reconnect_in_progress = true; }
  close() { this.closed = true; this.callback_close?.(); }
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
  _set_decorated(decorated) { this.decorated = decorated; }
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
  it('requests Xpra fullscreen state for the primary app while preserving dialog state', () => {
    const run = load();
    run(`const client = new XpraClient(800,600);
      const win = new XpraWindow(client, {x:0,y:0,w:800,h:600});
      client.send_configure_window(win, {focused:true}, true);`);
    expect(run('client.configuration')).toEqual({state:{focused:true,fullscreen:true},skip_geometry:true});
    run(`win.metadata = {'transient-for':1};
      client.send_configure_window(win, {focused:true}, false);`);
    expect(run('client.configuration')).toEqual({state:{focused:true},skip_geometry:false});
  });
  it('uses Xpra undecorated windows for embedded apps while preserving desktop mode', () => {
    const run = load();
    run(`const client = new XpraClient(800, 600);
      const win = new XpraWindow(client, {x:0,y:0,w:800,h:600});
      win._set_decorated(true);`);
    expect(run('win.decorated')).toBe(false);
    // Later metadata changes must not restore redundant window controls.
    run('win._set_decorated(true);');
    expect(run('win.decorated')).toBe(false);
    run(`client.server_is_desktop = true; win._set_decorated(true);`);
    expect(run('win.decorated')).toBe(true);
  });
  it('requests a fresh parent bootstrap from the embedded recovery control without redirecting', () => {
    const created: Array<{textContent?: string; onclick?: () => void; append: (...nodes: unknown[]) => void}> = [];
    const messages: unknown[] = [];
    const run = load({
      Worker: class {}, WebSocket: class {}, URL,
      location: {href: 'http://preview.localhost:19444/_capsem/surface/index.html', origin: 'http://preview.localhost:19444'},
      parent: {postMessage: (...args: unknown[]) => messages.push(args)},
      document: {
        querySelector: () => ({content: 'capsem-preview.secret', dataset: {parentOrigin: 'http://127.0.0.1:5173'}, remove: () => {}}),
        createElement: () => {const node = {append: () => {}}; created.push(node); return node;},
        body: {replaceChildren: () => {}},
      },
    });
    run('const client = new XpraClient(800, 600); client.callback_close = () => { throw new Error("vendor redirect"); }; client.connect(); client.close();');
    expect(created.map(node => node.textContent)).toEqual([undefined, 'App disconnected', 'Reconnect']);
    created[2].onclick!();
    expect(messages).toEqual([[{type: 'capsem:surface-reconnect'}, 'http://127.0.0.1:5173']]);
  });
  it('waits boundedly for initial transport startup and delegates after a successful open', () => {
    let now = 0;
    const run = load({Date: {now: () => now}});
    run('const client = new XpraClient(800, 600); client.connect(); client._process_error(["error", "not listening", 0]);');
    expect(run('client.retries')).toBe(1);
    run('client._process_close(["close", "closed"]);');
    expect(run('client.retries')).toBe(1);
    now = 60_001;
    run('client.reconnect_in_progress = false; client._process_error(["error", "not listening", 0]); client._process_close(["close", "closed"]);');
    expect(run('client.errors')).toBe(1);
    expect(run('client.closed')).toBe(true);
    run('client.connect();');
    expect(run('client.connects')).toBe(1);
    run('const opened = new XpraClient(800, 600); opened.connect(); opened._process_open(); opened._process_error(["error", "closed", 0]);');
    expect(run('opened.errors')).toBe(1);
    expect(run('opened.retries')).toBeUndefined();
    run('const denied = new XpraClient(800, 600); denied.connect(); denied._process_error(["error", "policy", 1008]);');
    expect(run('denied.errors')).toBe(1);
    expect(run('denied.retries')).toBeUndefined();
    run('denied._process_close(["close", "policy"]);');
    expect(run('denied.closed')).toBe(true);
  });
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
