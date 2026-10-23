import { expect, it, vi } from 'vitest';
import { BrowserHandoff } from '../models/browser-handoff.svelte';

const request = {id:1, url:'https://example.org/login'};

it('hides a retained action after opening it, and shows a fresh request', async () => {
  const handoff = new BrowserHandoff();
  const open = vi.fn().mockResolvedValue(undefined);
  expect(handoff.pending('vm', request)).toBe(true);
  await handoff.open('vm', request, open);
  expect(open).toHaveBeenCalledWith(request.url, 'vm');
  expect(handoff.pending('vm', {...request})).toBe(false);
  expect(handoff.pending('other', request)).toBe(true);
  expect(handoff.pending('vm', {...request, id:2})).toBe(true);
});

it('retains a failed action for retry and clears its error after success', async () => {
  const handoff = new BrowserHandoff();
  await handoff.open('vm', request, vi.fn().mockRejectedValue(new Error('failed')));
  expect(handoff.pending('vm', request)).toBe(true);
  expect(handoff.failed('vm', request)).toBe(true);
  expect(handoff.failed('other', request)).toBe(false);
  await handoff.open('vm', request, vi.fn().mockResolvedValue(undefined));
  expect(handoff.failed('vm', request)).toBe(false);
  expect(handoff.pending('vm', request)).toBe(false);
});

it('does not let a late failed handoff replace a newer accepted action', async () => {
  const handoff = new BrowserHandoff();
  let reject!: (error: Error) => void;
  const first = handoff.open('vm', request, () => new Promise((_resolve, fail) => { reject = fail; }));
  const latest = {...request, id:2};
  await handoff.open('vm', latest, vi.fn().mockResolvedValue(undefined));
  reject(new Error('old request'));
  await first;
  expect(handoff.pending('vm', latest)).toBe(false);
  expect(handoff.failed('vm', latest)).toBe(false);
});

it('never hands an unsafe request or detached session to the opener', async () => {
  const handoff = new BrowserHandoff();
  const open = vi.fn();
  await handoff.open(undefined, request, open);
  await handoff.open('vm', {...request, url:'javascript:alert(1)'}, open);
  expect(open).not.toHaveBeenCalled();
  expect(handoff.pending('vm', null)).toBe(false);
});
