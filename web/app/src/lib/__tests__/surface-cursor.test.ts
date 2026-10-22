import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { expect, it } from 'vitest';

const root = new URL('../../../../../crates/capsem-gateway/', import.meta.url);
const windowScript = readFileSync(new URL('vendor/xpra-html5/www/js/Window.js', root), 'utf8');
const policy = readFileSync(new URL('src/surface/static/capsem-surface-cursor.js', root), 'utf8');

function cursor(scale: number) {
  const styles: string[] = [];
  const context = vm.createContext({jQuery: () => ({css: (_key: string, value: string) => styles.push(value)})});
  vm.runInContext(windowScript, context);
  vm.runInContext(policy, context);
  context.scale = scale;
  vm.runInContext(`
    const win = Object.create(XpraWindow.prototype);
    win.wid = 1; win.client = {scale};
    win.construct_base64_image_url = () => 'data:image/png;base64,cursor';
  `, context);
  return {styles, run: (source: string) => vm.runInContext(source, context)};
}

it('uses native CSS image density without another canvas or cursor bitmap', () => {
  for (const scale of [1, 1.5, 2]) {
    const {styles, run} = cursor(scale);
    run('win.set_cursor("png", 64, 64, 12, 6, new Uint8Array())');
    expect(styles.at(-1)).toBe(`image-set(url("data:image/png;base64,cursor") ${scale}x) ${12 / scale} ${6 / scale}, auto`);
    // Remote pointer overlays still use physical coordinates inside the
    // transformed desktop; only the native CSS cursor needs density metadata.
    expect(run('win.cursor_data')).toEqual(['data:image/png;base64,cursor', 12, 6, 64, 64]);
  }
});

it('updates the current cursor when the desktop density changes', () => {
  const {styles, run} = cursor(2);
  run('win.set_cursor("png", 64, 64, 12, 6, new Uint8Array()); win.client.scale=1; win.update_cursor_density()');
  expect(styles.at(-1)).toContain('1x) 12 6, auto');
});

it('keeps the last cursor or reset without a stale asynchronous image load', () => {
  const {styles, run} = cursor(2);
  run('win.set_cursor("png",64,64,12,6,[]); win.set_cursor("png",32,32,0,0,[]); win.reset_cursor(); win.update_cursor_density()');
  expect(styles.at(-1)).toBe('default');
  expect(run('win.cursor_data')).toBeNull();
});
