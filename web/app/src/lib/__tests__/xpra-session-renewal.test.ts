import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { expect, it } from 'vitest';

const root = '../../../../../crates/capsem-gateway/';
const client = readFileSync(new URL(root + 'vendor/xpra-html5/www/js/Client.js', import.meta.url), 'utf8');
const policy = readFileSync(new URL(root + 'src/surface/static/capsem-surface-reconnect.js', import.meta.url), 'utf8');

function load(embedded = true) {
  const messages: unknown[] = [];
  const timers = new Map<number, () => void>();
  let timer = 0;
  const context = vm.createContext({
    window: {},
    document: {querySelector: () => embedded ? {dataset:{parentOrigin:'http://tauri.localhost'}} : null},
    parent: {postMessage: (...args: unknown[]) => messages.push(args)},
    setTimeout: (fn: () => void) => {timers.set(++timer,fn); return timer;},
    clearTimeout: (id: number) => timers.delete(id),
  });
  vm.runInContext(client, context);
  vm.runInContext(policy, context);
  const run = (code: string) => vm.runInContext(code, context);
  run(`const client = Object.create(XpraClient.prototype);
    Object.assign(client, {reconnect:true,reconnect_count:10,reconnect_attempt:0,closed:false,closes:0});
    client.log = client.clog = client.debug = () => {};
    for (const name of ['cancel_open_timer','cancel_hello_timer','cancel_all_files','emit_connection_lost',
      'remove_windows','close_audio','clear_timers','close_protocol','emit_connection_established'])
      client[name] = () => {};
    client.callback_close = () => client.closes++;`);
  return {run,messages,timers};
}

it('replaces an established embedded connection with one fresh bootstrap instead of retrying expired credentials', () => {
  const {run,messages,timers} = load();
  run(`client._process_startup_complete([]); client._process_close(['close','transport lost']);
    client.do_reconnect(); client._process_close(['close','late close']);`);
  expect(messages).toEqual([[{type:'capsem:surface-reconnect'},'http://tauri.localhost']]);
  expect(run('client.closed')).toBe(true);
  expect(run('client.closes')).toBe(1);
  expect(timers.size).toBe(0);
});

it('retains bounded initial startup retries and standalone reconnect behavior', () => {
  for (const embedded of [true,false]) {
    const {run,messages,timers} = load(embedded);
    if (!embedded) run('client._process_startup_complete([]);');
    run("client._process_close(['close','not ready']);");
    expect(messages).toEqual([]);
    expect(timers.size).toBe(1);
    run('client.close();');
    expect(timers.size).toBe(0);
  }
});

it('does not treat an explicit server disconnect as a transport retry', () => {
  const {run,messages} = load();
  run("client._process_startup_complete([]); client._process_disconnect(['disconnect','session ended']);");
  expect(messages).toEqual([]);
  expect(run('client.closed')).toBe(true);
});
