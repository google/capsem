// Exercise the pinned client's real scaling, DPI and pointer implementation.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { describe, expect, it } from 'vitest';

const root = '../../../../../crates/capsem-gateway/';
const read = (path: string) => readFileSync(new URL(root + path, import.meta.url), 'utf8');
const clientSource = read('vendor/xpra-html5/www/js/Client.js');
const windowSource = read('vendor/xpra-html5/www/js/Window.js');
const policy = read('src/surface/static/capsem-surface.js');

function load(ratio: number) {
  const viewport = {width:800,height:600};
  const observers: Array<{callback:()=>void; observing:boolean}> = [];
  class Observer {
    observing = false;
    constructor(public callback:()=>void) { observers.push(this); }
    observe() { this.observing = true; }
    disconnect() { this.observing = false; }
  }
  const context = vm.createContext({devicePixelRatio:ratio, viewport,
    ResizeObserver: Observer,
    window: {location:{href:'http://preview.localhost/'}},
    navigator: {userAgent:'test'}, METADATA_SUPPORTED: [],
    Utilities: {VERSION:'test',getSimpleUserAgentString:()=>'test'},
    document: {querySelector:(name:string) => name === '#dpi' ? {offsetWidth:96,offsetHeight:96} : null},
    jQuery: () => ({scrollLeft:()=>0,scrollTop:()=>0}),
    PACKET_TYPES: {configure_display:'configure-display',pointer_position:'pointer-position'},
  });
  for (const script of [clientSource,windowSource]) vm.runInContext(script,context);
  // Handshake DOM/protocol setup is unchanged; keep its delegation observable
  // while exercising the real geometry, DPI and mouse implementations.
  vm.runInContext(`XpraClient.prototype._process_hello = function(){ this.helloCalled = true; return 42; };
    XpraClient.prototype.close = function(){this.closed = true;};
    XpraClient.prototype._process_open = function(){this.opened = true;};`,context);
  vm.runInContext(policy,context);
  const run = (code:string) => vm.runInContext(code,context);
  run(`const client = Object.create(XpraClient.prototype);
    const style = {};
    const parent = {get clientWidth(){return viewport.width},get clientHeight(){return viewport.height}};
    client.container = {style,parentElement:parent,
      get clientWidth(){return Math.round(viewport.width * parseFloat(style.width || '100')/100)},
      get clientHeight(){return Math.round(viewport.height * parseFloat(style.height || '100')/100)}};
    Object.assign(client,{scale:1,desktop_width:800,desktop_height:600,connected:true,
      id_to_window:{1:{scale:1,screen_resized(){}}},sent:[],vrefresh:60});
    client.init_audio = client.init_packet_handlers = client.init_keyboard = client.position_float_menu = client.on_connection_progress = ()=>{};
    client._get_monitors = ()=>({});
    client._get_keyboard_layout = ()=>'us';
    client._get_encoding_caps = client._get_audio_caps = client._get_clipboard_caps = client._get_pointer_caps = client._get_file_caps = ()=>({});
    client._get_build_caps = client._get_platform_caps = client._get_network_caps = ()=>({});
    client.capabilities = {};
    client.send = packet=>client.sent.push(packet);
    client.init();`);
  return {run,context,viewport,observers};
}

describe('graphical HiDPI integration', () => {
  it('removes decorations through the real Xpra offset lifecycle and keeps pointer coordinates aligned', () => {
    const {run,context} = load(2);
    const header = {style: {display: 'block'}};
    context.document.getElementById = () => header;
    context.jQuery = () => ({
      css: (name: string) => name === 'height' ? '24' : '0',
      removeClass() { return this; }, addClass() { return this; },
      scrollLeft: () => 0, scrollTop: () => 0,
    });
    run(`client._make_hello(); client.debug_categories = [];
      const win = Object.create(XpraWindow.prototype);
      Object.assign(win, {client, wid:1, x:0, y:24, w:800, h:600, decorated:true,
        metadata:{}, override_redirect:false, tray:false, div:{},
        ensure_visible(){}, geometry_cb(){}, updateCSSGeometry(){}});
      win._set_decorated(true);
      CapsemSurface.fit(win);
      client._keyb_get_modifiers = ()=>[];
      client.on_mousemove({clientX:90,clientY:60,preventDefault(){}}, win);`);
    expect(header.style.display).toBe('none');
    expect(run('win.topoffset')).toBe(0);
    expect(run('win.get_internal_geometry()')).toMatchObject({x:0,y:0,w:1600,h:1200});
    expect(run('client.sent.at(-1)')).toEqual(['pointer-position',1,[180,120,180,120],[],[]]);
    run('client.send_configure_window(win, {focused:true}, true);');
    expect(run('client.sent.at(-1)[8]')).toEqual({focused:true,fullscreen:true});
  });

  it('uses physical backing pixels, negotiated DPI and the real upstream pointer transform', () => {
    const {run,viewport} = load(2);
    expect(run('client.container.clientWidth')).toBe(1600);
    expect(run('client.container.clientHeight')).toBe(1200);
    expect(run('client.container.style.transform')).toBe('scale(0.5)');
    expect(run('client._get_DPI()')).toBe(192);
    run('client._make_hello_base(); client._make_hello();');
    expect(run('client.capabilities.display.desktop_size')).toEqual([1600,1200]);
    expect(run('client.capabilities.display.desktop_mode_size')).toEqual([1600,1200]);
    expect(run('client.capabilities.dpi')).toEqual({'':192,x:192,y:192});
    expect(run('client.getMouse({clientX:315,clientY:177})')).toMatchObject({x:630,y:354});
    viewport.width = 900;
    run('client._screen_resized();');
    expect(run('client.sent.at(-1)[1]')).toMatchObject({'desktop-size':[1800,1200],dpi:{x:192,y:192}});
  });

  it('tracks fractional density and monitor changes without compounding scale', () => {
    const {run,context} = load(1.25);
    expect(run('client._get_DPI()')).toBe(120);
    expect(run('client.getMouse({clientX:80,clientY:40})')).toMatchObject({x:100,y:50});
    context.devicePixelRatio = 2;
    run('client._screen_resized(); client._screen_resized();');
    expect(run('client.scale')).toBe(2);
    expect(run('client.id_to_window[1].scale')).toBe(2);
    expect(run('client.container.clientWidth')).toBe(1600);
    context.devicePixelRatio = 1;
    run('client._screen_resized();');
    expect(run('client.container.style.transform')).toBe('scale(1)');
    expect(run('client.getMouse({clientX:80,clientY:40})')).toMatchObject({x:80,y:40});
  });

  it('keeps the previous dimensions while hidden and rejects invalid density', () => {
    const {run,context,viewport} = load(2);
    run('client._screen_resized(); client.sent = [];');
    viewport.width = 0; viewport.height = 0;
    context.devicePixelRatio = 1;
    run('client._screen_resized();');
    expect(run('client.desktop_width')).toBe(1600);
    expect(run('client.sent')).toEqual([]);
    viewport.width = 800; viewport.height = 600;
    context.devicePixelRatio = NaN;
    run('client._screen_resized();');
    expect(run('client.scale')).toBe(1);
    expect(run('client.container.clientWidth')).toBe(800);
  });

  it('respects the advertised physical limit without changing the pointer coordinate space', () => {
    const {run} = load(2);
    expect(run("client._process_hello(['hello',{max_desktop_size:[1000,600]}])")).toBe(42);
    expect(run('client.helloCalled')).toBe(true);
    expect(run('client.scale')).toBe(1);
    expect(run('client.container.clientHeight')).toBe(600);
    expect(run('client._get_DPI()')).toBe(96);
    expect(run('client.getMouse({clientX:80,clientY:40})')).toMatchObject({x:80,y:40});
    run("client._process_hello(['hello',{max_desktop_size:[0,Infinity]}]);");
    expect(run('client.scale')).toBe(1);
  });

  it('sends physical absolute and window-relative pointer positions using the vendor protocol path', () => {
    const {run} = load(2);
    run(`client._keyb_get_modifiers = ()=>[];
      client.on_mousemove({clientX:90,clientY:60,preventDefault(){}},
        {wid:1,get_internal_geometry:()=>({x:100,y:50})});`);
    expect(run('client.sent.at(-1)')).toEqual(['pointer-position',1,[180,120,80,70],[],[]]);
  });

  it('refits after a hidden view becomes visible and disconnects its observer with the client', () => {
    const {run,viewport,observers} = load(2);
    run('client._make_hello();');
    expect(observers).toHaveLength(1);
    viewport.width = 0; viewport.height = 0;
    observers[0].callback();
    expect(run('client.desktop_width')).toBe(1600);
    viewport.width = 900; viewport.height = 700;
    observers[0].callback();
    expect(run('client.sent.at(-1)[1]["desktop-size"]')).toEqual([1800,1400]);
    run('client.close();');
    expect(observers[0].observing).toBe(false);
    expect(run('client.closed')).toBe(true);
    run('client._process_open();');
    expect(observers).toHaveLength(2);
    expect(observers[1].observing).toBe(true);
  });
});
