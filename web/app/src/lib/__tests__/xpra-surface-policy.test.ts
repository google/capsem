// capsem-surface.js is served by the gateway with the pinned xpra-html5
// client. It runs here against stand-ins for the client's two classes, with
// the same classic-script global scope a browser gives it.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { describe, expect, it } from 'vitest';

const POLICY = readFileSync(
  new URL('../../../../../crates/capsem-gateway/src/surface/assets/capsem-surface.js', import.meta.url),
  'utf8',
);

// The client's classes, reduced to what the policy touches.
const CLIENT = `
class XpraClient {
  constructor(width, height) { this.desktop_width = width; this.desktop_height = height; this.capabilities = {}; }
  _get_desktop_size() { return [this.desktop_width, this.desktop_height]; }
  _make_hello() { this.capabilities.system_tray = true; this.capabilities.windows = true; }
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

function load() {
  const context = vm.createContext({});
  vm.runInContext(CLIENT, context);
  vm.runInContext(POLICY, context);
  return (source: string) => vm.runInContext(source, context);
}

describe('capsem-surface.js', () => {
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
