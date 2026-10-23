// Exercise keyboard initialization in the actual pinned Xpra client. Browser
// layout discovery can finish after the socket has connected.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { expect, it } from 'vitest';

const source = readFileSync(new URL('../../../../../crates/capsem-gateway/vendor/xpra-html5/www/js/Client.js', import.meta.url), 'utf8');

it('restores DOM keyboard focus on guest clicks even when the guest window already has focus', () => {
  let focused = 0;
  const context = vm.createContext({ window: {}, document: { getElementById: (id: string) => {
    expect(id).toBe('pasteboard');
    return { focus: (options: unknown) => { expect(options).toEqual({preventScroll:true}); focused += 1; } };
  } } });
  vm.runInContext(source, context);
  vm.runInContext(`const client = Object.create(XpraClient.prototype);
    client.connected = true; client.focused_wid = 42;
    client.do_window_mouse_click = () => {};
    client.on_mousedown({}, {wid:42});`, context);
  expect(focused).toBe(1);
  vm.runInContext(`client.on_mousedown({}, null);
    client.server_readonly = true; client.on_mousedown({}, {wid:42});
    client.server_readonly = false; client.connected = false; client.on_mousedown({}, {wid:42});`, context);
  expect(focused).toBe(1);
});

it('does not disable connected keyboard capture when layout discovery finishes late', async () => {
  const handlers = new Map<string, unknown>();
  const context = vm.createContext({ window: {}, navigator: {}, clog() {}, cdebug() {}, document: { addEventListener: (name: string, handler: unknown) => handlers.set(name, handler) } });
  vm.runInContext(source, context);
  vm.runInContext(`const client = Object.create(XpraClient.prototype);
    const layout = new Promise(resolve => globalThis.finishLayout = resolve);
    navigator.keyboard = {getLayoutMap: () => layout};
    client.capture_keyboard = false;
    client.query_keyboard_map();
    client.connected = true;
    client.capture_keyboard = true;
    finishLayout(new Map());`, context);
  await vm.runInContext('layout', context);
  expect(vm.runInContext('client.capture_keyboard', context)).toBe(true);
  expect([...handlers.keys()]).toEqual(['keydown', 'keyup']);
});

it('preserves deliberately disabled capture during keyboard initialization', () => {
  const context = vm.createContext({ window: {}, document: { addEventListener() {} } });
  vm.runInContext(source, context);
  vm.runInContext(`const client = Object.create(XpraClient.prototype);
    client.capture_keyboard = false;
    client.do_init_keyboard();`, context);
  expect(vm.runInContext('client.capture_keyboard', context)).toBe(false);
});
