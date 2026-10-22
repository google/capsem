// Exercise keyboard initialization in the actual pinned Xpra client. Browser
// layout discovery can finish after the socket has connected.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { expect, it } from 'vitest';

const source = readFileSync(new URL('../../../../../crates/capsem-gateway/vendor/xpra-html5/www/js/Client.js', import.meta.url), 'utf8');

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
