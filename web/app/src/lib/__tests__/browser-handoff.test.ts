import { expect, it, vi } from 'vitest';
import { BrowserHandoff } from '../models/browser-handoff.svelte';

const request = {owner_generation:'18f59e33-8af6-4992-81d9-71e10873af9a', id:1, url:'https://example.org/login'};

it('acknowledges metadata after success and accepts a reused ID from a new owner', async () => {
  const handoff = new BrowserHandoff();
  const open=vi.fn().mockResolvedValue(undefined), ack=vi.fn().mockResolvedValue(undefined);
  await handoff.open('vm',request,open,ack);
  expect(ack).toHaveBeenCalledExactlyOnceWith('vm',{owner_generation:request.owner_generation,request_id:1});
  expect(handoff.pending('vm',{...request,owner_generation:'ab60a28c-594c-45bb-8cc6-161626b35eef'})).toBe(true);
});

it('retries a failed confirmation without ever reopening the browser', async () => {
  const handoff = new BrowserHandoff();
  const open=vi.fn().mockResolvedValue(undefined), ack=vi.fn().mockRejectedValue({status:503});
  await handoff.open('vm',request,open,ack);
  expect(ack).toHaveBeenCalledTimes(3);
  expect(handoff.pending('vm',request)).toBe(false);
  expect(handoff.confirmationFailed('vm',request)).toBe(true);
  await handoff.open('vm',request,open,ack);
  expect(open).toHaveBeenCalledTimes(1);
  ack.mockResolvedValue(undefined);
  await handoff.retryConfirmation('vm',request,ack);
  expect(open).toHaveBeenCalledTimes(1);
  expect(handoff.confirmationFailed('vm',request)).toBe(false);
});

it('deduplicates an in-flight opener and never acknowledges its failure', async () => {
  const handoff=new BrowserHandoff();let fail!:(error:Error)=>void;
  const open=vi.fn(()=>new Promise<void>((_resolve,reject)=>{fail=reject;})),ack=vi.fn();
  const first=handoff.open('vm',request,open,ack);
  void handoff.open('vm',request,open,ack);
  expect(open).toHaveBeenCalledTimes(1);
  fail(new Error('failed'));await first;
  expect(ack).not.toHaveBeenCalled();
  expect(handoff.failed('vm',request)).toBe(true);
});

it('does not let a late failed acknowledgement overwrite a newer owner',async()=>{
  const handoff=new BrowserHandoff();let fail!:(error:unknown)=>void;
  const old=handoff.open('vm',request,vi.fn().mockResolvedValue(undefined),()=>new Promise((_resolve,reject)=>{fail=reject;}));
  await Promise.resolve();await Promise.resolve();
  const next={...request,owner_generation:'ab60a28c-594c-45bb-8cc6-161626b35eef'};
  await handoff.open('vm',next,vi.fn().mockResolvedValue(undefined),vi.fn().mockResolvedValue(undefined));
  fail({status:409});await old;
  expect(handoff.pending('vm',next)).toBe(false);
  expect(handoff.confirmationFailed('vm',next)).toBe(false);
});

it('hides a retained action after opening it, and shows a fresh request', async () => {
  const handoff = new BrowserHandoff();
  const open = vi.fn().mockResolvedValue(undefined);
  expect(handoff.pending('vm', request)).toBe(true);
  await handoff.open('vm', request, open, vi.fn().mockResolvedValue(undefined));
  expect(open).toHaveBeenCalledWith(request.url, 'vm');
  expect(handoff.pending('vm', {...request})).toBe(false);
  expect(handoff.pending('other', request)).toBe(true);
  expect(handoff.pending('vm', {...request, id:2})).toBe(true);
});

it('retains a failed action for retry and clears its error after success', async () => {
  const handoff = new BrowserHandoff();
  await handoff.open('vm', request, vi.fn().mockRejectedValue(new Error('failed')), vi.fn());
  expect(handoff.pending('vm', request)).toBe(true);
  expect(handoff.failed('vm', request)).toBe(true);
  expect(handoff.failed('other', request)).toBe(false);
  await handoff.open('vm', request, vi.fn().mockResolvedValue(undefined), vi.fn().mockResolvedValue(undefined));
  expect(handoff.failed('vm', request)).toBe(false);
  expect(handoff.pending('vm', request)).toBe(false);
});

it('does not let a late failed handoff replace a newer accepted action', async () => {
  const handoff = new BrowserHandoff();
  let reject!: (error: Error) => void;
  const first = handoff.open('vm', request, () => new Promise((_resolve, fail) => { reject = fail; }), vi.fn());
  const latest = {...request, id:2};
  await handoff.open('vm', latest, vi.fn().mockResolvedValue(undefined), vi.fn().mockResolvedValue(undefined));
  reject(new Error('old request'));
  await first;
  expect(handoff.pending('vm', latest)).toBe(false);
  expect(handoff.failed('vm', latest)).toBe(false);
});

it('never hands an unsafe request or detached session to the opener', async () => {
  const handoff = new BrowserHandoff();
  const open = vi.fn();
  await handoff.open(undefined, request, open, vi.fn().mockResolvedValue(undefined));
  await handoff.open('vm', {...request, url:'javascript:alert(1)'}, open, vi.fn().mockResolvedValue(undefined));
  expect(open).not.toHaveBeenCalled();
  expect(handoff.pending('vm', null)).toBe(false);
});
