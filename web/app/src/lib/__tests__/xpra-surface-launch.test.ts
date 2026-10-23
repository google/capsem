import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { expect, it, vi } from 'vitest';

const script = readFileSync(new URL('../../../../../crates/capsem-gateway/src/surface/static/launch.js', import.meta.url), 'utf8');

async function launch(parent = '', profile = 'auto', session: unknown = {
  url: 'http://preview.localhost:19444/_capsem/bootstrap', bootstrap_token: 'single-use-secret',
}) {
  const nodes = {
    status: {textContent: ''}, open: {disabled: false, hidden: false, addEventListener: vi.fn()}, progress: {hidden: true},
  };
  const children: {type: string; name: string; value: string}[] = [];
  let submitted = false;
  const form = {method: '', action: '', append: (input: typeof children[number]) => children.push(input), submit: () => {submitted = true;}};
  const fetch = vi.fn();
  const replaceState = vi.fn();
  const context = vm.createContext({
    URL, fetch, history: {replaceState}, location: {pathname: '/vms/vm-1/surface/', search: '', hash: '#' + encodeURIComponent(JSON.stringify(session))},
    setTimeout: (done: () => void) => {done();},
    document: {
      getElementById: (id: keyof typeof nodes) => nodes[id],
      createElement: (name: string) => name === 'form' ? form : {type: '', name: '', value: ''},
      body: {dataset: {auto: 'true', parent, profile}, append: vi.fn()},
    },
  });
  vm.runInContext(script, context);
  await vi.waitFor(() => expect(submitted || nodes.status.textContent.startsWith('Could not')).toBe(true));
  return {nodes, form, children, submitted, fetch, replaceState};
}

it('posts only the pre-minted scoped bootstrap into the current frame', async () => {
  const result = await launch('http://127.0.0.1:5173');
  expect(result.submitted).toBe(true);
  expect(result.fetch).not.toHaveBeenCalled();
  expect(result.replaceState).toHaveBeenCalledWith(null, '', '/vms/vm-1/surface/');
  expect(result.form.method).toBe('POST');
  expect(result.form.action).toBe('http://preview.localhost:19444/_capsem/surface/index.html');
  expect(result.children.map(({name, value}) => [name, value])).toEqual([
    ['parent_origin', 'http://127.0.0.1:5173'], ['display_profile', 'auto'], ['bootstrap_token', 'single-use-secret'],
  ]);
  expect(JSON.stringify(result.form)).not.toContain('secret');
  expect(result.nodes.progress.hidden).toBe(false);
});

it('carries the selected viewer profile through the authenticated form', async () => {
  const result = await launch('http://127.0.0.1:5173', 'remote');
  expect(result.children.find(input => input.name === 'display_profile')?.value).toBe('remote');
});

it('refuses an untrusted bootstrap destination without any bearer fetch', async () => {
  const result = await launch('', 'auto', {url: 'https://evil.example/_capsem/bootstrap', bootstrap_token: 'scoped'});
  expect(result.submitted).toBe(false);
  expect(result.fetch).not.toHaveBeenCalled();
  expect(result.nodes.open.disabled).toBe(false);
  expect(result.nodes.open.hidden).toBe(false);
  expect(result.nodes.progress.hidden).toBe(true);
});

it('keeps standalone cookie bootstrapping on its existing path', async () => {
  const result = await launch();
  expect(result.form.action).toBe('http://preview.localhost:19444/_capsem/bootstrap');
  expect(result.children.map(input => input.name)).toEqual(['bootstrap_token']);
});
