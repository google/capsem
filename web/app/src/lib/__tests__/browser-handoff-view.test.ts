import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
vi.mock('../api', () => ({openUrl:vi.fn(),acknowledgeBrowserRequest:vi.fn()}));
import BrowserRequest from '../components/shell/BrowserRequest.svelte';

afterEach(() => vi.unstubAllGlobals());
const props = {vmId:'vm', request:{id:1, url:'https://example.org/login'}};
const source = readFileSync(
  new URL('../components/shell/BrowserRequest.svelte', import.meta.url),
  'utf8',
);

it('requires an explicit browser gesture for native clients', () => {
  vi.stubGlobal('window', {__TAURI_INTERNALS__:{}});
  const body = render(BrowserRequest, {props}).body;
  expect(body).toContain('Open browser');
  expect(body).toContain('Open https://example.org/login on this device');
  expect(source).not.toContain('$effect(');
  expect(source).not.toContain('BrowserRequestCursor');
  expect(source).toMatch(/onclick=\{\(\) => \{ void handoff\.open\(/u);
});

it('offers the explicit browser gesture for web clients', () => {
  vi.stubGlobal('window', {});
  expect(render(BrowserRequest, {props}).body).toContain('Open browser');
});
