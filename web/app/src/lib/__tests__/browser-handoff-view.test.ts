import { afterEach, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
vi.mock('../api', () => ({openUrl:vi.fn(),acknowledgeBrowserRequest:vi.fn()}));
import BrowserRequest from '../components/shell/BrowserRequest.svelte';

afterEach(() => vi.unstubAllGlobals());
const props = {vmId:'vm', request:{id:1, url:'https://example.org/login'}};

it('keeps a retained native browser URL out of the session toolbar', () => {
  vi.stubGlobal('window', {__TAURI_INTERNALS__:{}});
  expect(render(BrowserRequest, {props}).body).not.toContain('Open browser');
});

it('offers the explicit browser gesture for web clients', () => {
  vi.stubGlobal('window', {});
  expect(render(BrowserRequest, {props}).body).toContain('Open browser');
});
