// Exercise the real pinned Client.js. Only browser/platform boundaries and
// transport side effects are replaced; reset, retry and close stay upstream.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { expect, it } from 'vitest';

const root = '../../../../../crates/capsem-gateway/';
const client = readFileSync(new URL(root + 'vendor/xpra-html5/www/js/Client.js', import.meta.url), 'utf8');
const policy = readFileSync(new URL(root + 'src/surface/static/capsem-surface.js', import.meta.url), 'utf8');
const windowSource = readFileSync(new URL(root + 'vendor/xpra-html5/www/js/Window.js', import.meta.url), 'utf8');

it.each(['mousedown', 'mouseup', 'mousemove'])('preserves the canvas %s callback propagation decision', event => {
  const handlers = new Map<string, (event: object) => unknown>();
  const context = vm.createContext({
    jQuery: () => Object.fromEntries(['mousedown', 'mouseup', 'mousemove'].map(name =>
      [name, (callback: (event: object) => unknown) => handlers.set(name, callback)])),
  });
  vm.runInContext(windowSource, context);
  vm.runInContext(`
    globalThis.propagate = false;
    const win = Object.create(XpraWindow.prototype);
    win.mouse_down_cb = win.mouse_up_cb = win.mouse_move_cb = () => propagate;
    win.register_canvas_mouse_events({});
  `, context);
  // jQuery consumes false to stop propagation. Dropping it sends the same
  // browser event through both the window callback and the screen listener.
  expect(handlers.get(event)?.({})).toBe(false);
  vm.runInContext('propagate = true;', context);
  expect(handlers.get(event)?.({})).toBe(true);
});

function load() {
  const timers = new Map<number, () => void>();
  const contexts: {closed: boolean; close: () => void}[] = [];
  const parent = {clientWidth: 800, clientHeight: 600};
  const screen = Object.assign(new EventTarget(), {clientWidth: 800, clientHeight: 600, style: {}, parentElement: parent});
  let present = true;
  let clock = 0;
  let timerId = 0;
  const document = {
    getElementById: (name: string) => name === 'screen' && present ? screen : null,
    querySelector: (name: string) => name === '#screen' && present ? screen : null,
  };
  const context = vm.createContext({
    document, window: {}, console, Event, setTimeout: (fn: () => void) => {timers.set(++timerId, fn); return timerId;},
    clearTimeout: (id: number) => timers.delete(id),
    Date: class extends Date {static now() {return clock;}},
    Utilities: {
      isMacOS: () => false, getHexUUID: () => 'test', getColorGamut: () => 'srgb',
      getFirstBrowserLanguage: () => 'en-US', isEventSupported: () => true, clog: () => {},
    },
    default_settings: {}, lz4: {decode: () => {}}, MediaSourceUtil: {getMediaSourceClass: () => undefined},
    AudioContext: class {
      closed = false;
      constructor() {contexts.push(this);}
      close() {this.closed = true; return Promise.resolve();}
    },
    jQuery: () => ({
      mousedown: (fn: EventListener) => screen.addEventListener('mousedown', fn),
      mouseup: (fn: EventListener) => screen.addEventListener('mouseup', fn),
    }),
  });
  vm.runInContext(client, context);
  vm.runInContext(`
    class XpraWindow { screen_resized() {} move_resize() {} }
    // Transport and window cleanup are boundaries; the actual reconnect
    // scheduler, state reset, input binding and close methods are unchanged.
    for (const name of ['remove_windows','close_audio','cancel_all_files','clear_timers','close_protocol','emit_connection_lost'])
      XpraClient.prototype[name] = function () {};
    XpraClient.prototype.connect = function () {this.connects = (this.connects || 0) + 1;};
  `, context);
  vm.runInContext(policy, context);
  const run = (source: string) => vm.runInContext(source, context);
  run('globalThis.testClient = new XpraClient("screen");');
  return {
    run, screen, timers, contexts,
    advance: (now: number) => {clock = now;}, removeScreen: () => {present = false;},
    tick: () => {
      const ready = [...timers.values()]; timers.clear();
      for (const fn of ready) fn();
    },
  };
}

it('cancels delayed reconnect before terminal close removes the drawing DOM', () => {
  const fixture = load();
  fixture.run('testClient.connect(); testClient.do_reconnect();');
  expect(fixture.timers.size).toBe(1);
  fixture.advance(60_001);
  fixture.run('testClient.connect();'); // Capsem deadline ends initial startup.
  fixture.removeScreen();
  expect(() => fixture.tick()).not.toThrow();
  expect(fixture.timers.size).toBe(0);
  expect(fixture.run('testClient.connects')).toBe(1);
});

it('binds pointer input once across repeated connection-state resets', () => {
  const fixture = load();
  fixture.run(`
    globalThis.counts = {mousedown: 0, mouseup: 0, mousemove: 0, wheel: 0};
    for (const [event, method] of Object.entries({mousedown:'on_mousedown',mouseup:'on_mouseup',mousemove:'on_mousemove',wheel:'on_mousescroll'}))
      testClient[method] = () => counts[event]++;
    for (let n = 0; n < 4; n++) testClient.init_state();
  `);
  for (const event of ['mousedown','mouseup','mousemove','wheel']) fixture.screen.dispatchEvent(new Event(event));
  expect(fixture.run('counts')).toEqual({mousedown: 1, mouseup: 1, mousemove: 1, wheel: 1});
});

it('owns one bell audio context until the client is finally closed', () => {
  const fixture = load();
  fixture.run('for (let n = 0; n < 4; n++) testClient.init_state();');
  expect(fixture.contexts).toHaveLength(1);
  fixture.run('testClient.close();');
  expect(fixture.contexts[0].closed).toBe(true);
});

it('keeps only one scheduled reconnect and lets an explicit close cancel it', () => {
  const fixture = load();
  fixture.run('testClient.do_reconnect(); testClient.do_reconnect();');
  expect(fixture.timers.size).toBe(1);
  fixture.run('testClient.close();');
  expect(fixture.timers.size).toBe(0);
  expect(fixture.run('testClient.reconnect_in_progress')).toBe(false);
});

it('ignores a late transport close after the client has been closed', () => {
  const fixture = load();
  fixture.run('testClient.close(); testClient._process_close(["close", "late packet"]);');
  expect(fixture.timers.size).toBe(0);
  expect(() => fixture.tick()).not.toThrow();
});

it('runs final disconnect cleanup and its close callback exactly once', () => {
  const fixture = load();
  fixture.run(`
    globalThis.closes = 0;
    testClient.callback_close = () => closes++;
    testClient._process_disconnect(['disconnect', 'finished']);
    testClient.close();
  `);
  expect(fixture.run('closes')).toBe(1);
  expect(fixture.contexts[0].closed).toBe(true);
});

it('still reconnects repeatedly while retaining one pointer handler and audio context', () => {
  const fixture = load();
  fixture.run('testClient.connect(); globalThis.moves = 0; testClient.on_mousemove = () => moves++;');
  for (let n = 0; n < 4; n++) {
    fixture.run('testClient.do_reconnect();');
    fixture.tick();
  }
  fixture.screen.dispatchEvent(new Event('mousemove'));
  expect(fixture.run('testClient.connects')).toBe(5);
  expect(fixture.run('testClient.reconnect_in_progress')).toBe(false);
  expect(fixture.run('moves')).toBe(1);
  expect(fixture.contexts).toHaveLength(1);
});
