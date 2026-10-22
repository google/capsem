// Run the pinned renderer. jQuery is the DOM boundary: guest data must reach
// text/attribute APIs, never an HTML constructor or a CSS selector.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { expect, it } from 'vitest';

const source = readFileSync(new URL('../../../../../crates/capsem-gateway/vendor/xpra-html5/www/js/Notifications.js', import.meta.url), 'utf8');

function load() {
  const context = vm.createContext({ console: {debug() {}} });
  vm.runInContext(`
    const markup = [], texts = [], attributes = [], lookups = [], events = [];
    const window = {}, document = {getElementById(id) {lookups.push(id); return {};}};
    function $(value) {
      if (typeof value === 'function') return value();
      if (typeof value === 'string') markup.push(value);
      const node = {
        handlers: {},
        text(value) {texts.push(value); return this;},
        attr(key, value) {attributes.push([key, value]); return this;},
        addClass() {return this;}, removeClass() {return this;},
        append() {return this;}, prepend() {return this;},
        find(selector) {return $(selector);},
        on(name, selector, handler) {this.handlers[name] = handler || selector; return this;},
        trigger(name) {events.push(name); this.handlers[name]?.(); return this;},
      };
      return node;
    }
    $.debounce = (_delay, callback) => callback;
    function setTimeout() {}
  `, context);
  vm.runInContext(source, context);
  return (code: string) => vm.runInContext(code, context);
}

it('keeps guest title, body, action label and identifiers out of HTML and selectors', () => {
  const run = load();
  const payload = '<img src=x onerror="alert(1)"><button>Forged control</button>';
  const nid = '"><img src=x> #screen, .notifications';
  const action = '" onclick="alert(1)';
  run(`window.doNotification('info', ${JSON.stringify(nid)}, ${JSON.stringify(payload)}, ${JSON.stringify(payload)}, 0, null, [${JSON.stringify(action)}, ${JSON.stringify(payload)}], {}, () => {}, () => {});`);
  expect(run('texts')).toEqual(expect.arrayContaining([payload, payload, payload]));
  for (const markup of run('markup') as string[]) {
    expect(markup).not.toContain(payload);
    expect(markup).not.toContain(nid);
    expect(markup).not.toContain(action);
  }
  run(`window.closeNotification(${JSON.stringify(nid)});`);
  expect(run('lookups')).toEqual(['notification' + nid]);
});

it('keeps exact action identity and close callbacks when the safe button is clicked', () => {
  const run = load();
  run(`
    window.closeNotification = id => events.push(['dismiss', id]);
    window._notification_button(42, 'open:original', 'Open & inspect',
      (...args) => events.push(['action', ...args]),
      (...args) => events.push(['close', ...args])).trigger('click');
  `);
  expect(run('texts')).toContain('Open & inspect');
  expect(run('events')).toEqual(['click', ['dismiss', 42], ['action', 42, 'open:original'], ['close', 42, 3, 'user clicked action']]);
});
