import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { describe, expect, it } from 'vitest';
import { metricMs, metricSize, parseSurfaceMetrics } from '../models/surface-metrics';

const SCRIPT = readFileSync(new URL('../../../../../crates/capsem-gateway/src/surface/static/capsem-surface-metrics.js', import.meta.url), 'utf8');

function load() {
  const listeners: Record<string, (event: unknown) => void> = {};
  const messages: unknown[] = [];
  let clock = 0;
  let timer: (() => void) | undefined;
  const parent = { postMessage: (message: unknown) => messages.push(message) };
  const context = vm.createContext({
    performance: { now: () => clock }, parent, devicePixelRatio: 2, TextDecoder, Uint8Array,
    PACKET_TYPES: {info_request:'info-request'},
    document: { hidden: false, querySelector: () => ({dataset: {parentOrigin: 'http://parent.localhost'}}),
      addEventListener: (name: string, listener: (e: unknown) => void) => {listeners[name] = listener;} },
    addEventListener: (name: string, listener: (e: unknown) => void) => {listeners[name] = listener;},
    setInterval: (callback: () => void) => {timer = callback; return 1;},
    clearInterval: () => {timer = undefined;},
  });
  vm.runInContext(`class XpraClient {
    _process_open() {} _process_close() {} _process_draw() {}
    draw_pending_list() { this.pending_redraw = []; }
    do_send_damage_sequence() {}
    send(packet) { (this.sent ||= []).push(packet); }
  }`, context);
  vm.runInContext(SCRIPT, context);
  const run = (code: string) => vm.runInContext(code, context);
  const toggle = (enabled: boolean, origin = 'http://parent.localhost', source: unknown = parent) =>
    listeners.message({origin, source, data: {type: 'capsem:surface-debug', enabled}});
  return {run, messages, listeners, toggle, tick: (now: number) => {clock = now; timer?.();}, hasTimer: () => Boolean(timer)};
}

describe('graphical display diagnostics', () => {
  it('counts presented frames, reports zero at idle, and bounds retained timing samples', () => {
    const {run} = load();
    run('const meter = new CapsemSurfaceMetrics(0); meter.frame(100, 2); meter.frame(200, 3);');
    expect(run('meter.sample({}, 1000).fps')).toBe(2);
    expect(run('meter.sample({}, 2001).fps')).toBe(0);
    expect(run('meter.sample({}, 2001).frameMs')).toBeNull();
    run('for(let n=0;n<10000;n++) meter.frame(2100+n/100, 1);');
    expect(run('meter.frames.length')).toBeLessThanOrEqual(512);
  });

  it('keeps wire format distinct from display dimensions and never invents depth or quality', () => {
    const {run} = load();
    run(`const meter = new CapsemSurfaceMetrics(0);
      const client = {focused_wid: 1, id_to_window: {1: {w: 1920, h: 1080, metadata: {depth: 24},
        canvas: {width: 1920, height: 1080, getBoundingClientRect: () => ({width: 960, height: 540})}}}};
      meter.draw(['draw',1,0,0,200,100,'webp',new Uint8Array(300),1,0,{quality:85,subsampling:'YUV420P'}]);`);
    expect(run('meter.sample(client, 1000)')).toMatchObject({codec:'webp',quality:85,color:'YUV420P',depth:24,
      sourceWidth:1920,sourceHeight:1080,backingWidth:1920,backingHeight:1080,displayWidth:960,displayHeight:540,dpr:2});
    run("meter.draw(['draw',1,0,0,1,1,'png',new Uint8Array(),2,0,{}]);");
    expect(run('meter.sample({}, 1000)')).toMatchObject({quality:null,depth:null,color:null,rttMs:null});
    run("meter.draw(['draw',1,0,0,1,1,new Uint8Array([114,103,98]),new Uint8Array(),3,0,{rgb_format:new Uint8Array([82,71,66])}]);");
    expect(run('meter.sample({}, 1000)')).toMatchObject({codec:'rgb',color:'RGB'});
  });

  it('measures oldest input to next paint, discards stale input and marks offscreen paint FPS unavailable', () => {
    const {run} = load();
    run("const meter = new CapsemSurfaceMetrics(0); meter.input('mouse',100); meter.input('mouse',120); meter.frame(150,2);");
    expect(run('meter.sample({}, 1000).mouseMs')).toBe(50);
    run("meter.input('mouse',200); meter.frame(2000,2);");
    expect(run('meter.sample({}, 2000).mouseMs')).toBeNull();
    expect(run('meter.sample({offscreen_api:true,decode_worker:{}}, 2000).fps')).toBeNull();
    run('meter.ack(-1);');
    expect(run('meter.sample({},2000).decodeErrors')).toBe(1);
  });

  it('only enables from the trusted parent, stays inert by default and tears down on close', () => {
    const {run,toggle,tick,messages,hasTimer} = load();
    run('const client = new XpraClient(); client._process_open();');
    expect(hasTimer()).toBe(false);
    toggle(true, 'http://hostile.localhost');
    toggle(true, 'http://parent.localhost', {});
    expect(hasTimer()).toBe(false);
    toggle(true);
    expect(hasTimer()).toBe(true);
    tick(100);
    run('client.pending_redraw = [1,2,3]; client.draw_pending_list();');
    tick(1000);
    expect(messages).toContainEqual(expect.objectContaining({type:'capsem:surface-metrics', metrics:expect.objectContaining({fps:1})}));
    toggle(false);
    expect(hasTimer()).toBe(false);
    toggle(true);
    run('client._process_close();');
    expect(hasTimer()).toBe(false);
  });

  it('validates every iframe metric before rendering and formats unavailable values honestly', () => {
    const {run} = load();
    const row = run('new CapsemSurfaceMetrics(0).sample({},1000)');
    expect(parseSurfaceMetrics(row)).not.toBeNull();
    for (const bad of [null, [], {}, {...row,fps:Infinity}, {...row,mouseMs:-1}, {...row,codec:'x'.repeat(33)}, {...row,dpr:undefined}, {...row,guestDpi:Infinity}]) {
      expect(parseSurfaceMetrics(bad)).toBeNull();
    }
    expect(metricMs(null)).toBe('—');
    expect(metricMs(12.345)).toBe('12.3 ms');
    expect(metricSize(1280,720)).toBe('1280 × 720');
    expect(metricSize(null,720)).toBe('—');
  });

  it('requests only display information at a bounded frequency while enabled', () => {
    const {run,toggle,tick} = load();
    run('const client = new XpraClient(); client.connected=true; client.uuid="test"; client._process_open();');
    toggle(true);
    tick(500); tick(1500); tick(2000);
    expect(run('client.sent')).toEqual([
      ['info-request',['test'],[],['displaymanager','x11displaymanager']],
      ['info-request',['test'],[],['displaymanager','x11displaymanager']],
    ]);
    run('client.server_last_info = {display:{depth:24,dpi:{value:192}}}; client.capabilities={dpi:{"":192}};');
    expect(run('new CapsemSurfaceMetrics(0).sample(client,2000)')).toMatchObject({depth:24,clientDpi:192,guestDpi:192});
    toggle(false); tick(5000);
    expect(run('client.sent.length')).toBe(2);
  });
});
