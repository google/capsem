import {describe, expect, it, vi} from 'vitest';
import {SecurityAskStatus, SecurityAskDecision, type SecurityAskListResponse} from '@capsem/sdk';
import {ApprovalQueue, ApprovalNotices, ApprovalRefresh, approvalKey} from '../models/security-asks';
import {render} from 'svelte/server';
import Panel from '../components/shell/ApprovalPanel.svelte';

const response: SecurityAskListResponse = {vm_id:'vm-a', generation:'generation-a', pending_count:1,
  requests:[{ask_id:'ask-a',event_id:'event-a',event_type:'http',rule_id:'rule-a',rule_name:'Approval policy',
    summary:'HTTPS request needs approval',status:SecurityAskStatus.PENDING,created_at_unix_ms:1,
    expires_at_unix_ms:10000}]};

describe('generation-bound approval actions',()=>{
  it('uses the listed generation once and refreshes a conflict without replaying the decision',async()=>{
    const read=vi.fn().mockResolvedValue(response);
    const resolve=vi.fn().mockRejectedValue({status:409});
    const queue=new ApprovalQueue(read,resolve,()=>{},()=>100);
    await queue.load('vm-a');
    await queue.decide('ask-a',SecurityAskDecision.APPROVE);
    expect(resolve).toHaveBeenCalledExactlyOnceWith('vm-a','ask-a','generation-a',SecurityAskDecision.APPROVE);
    expect(read).toHaveBeenCalledTimes(2);
    expect(queue.state.error).toMatch(/changed/);
  });
  it('refuses expired, resolved and duplicate in-flight actions',async()=>{
    let finish!:()=>void;
    const resolve=vi.fn(()=>new Promise<void>(done=>{finish=done;}));
    const read=vi.fn().mockResolvedValue(response);
    const queue=new ApprovalQueue(read,resolve,()=>{},()=>100);
    await queue.load('vm-a');
    const first=queue.decide('ask-a',SecurityAskDecision.DENY);
    await queue.decide('ask-a',SecurityAskDecision.APPROVE);
    expect(resolve).toHaveBeenCalledTimes(1);
    finish();await first;
    const expired=new ApprovalQueue(read,resolve,()=>{},()=>10001);
    await expired.load('vm-a');await expired.decide('ask-a',SecurityAskDecision.APPROVE);
    expect(resolve).toHaveBeenCalledTimes(1);
    read.mockResolvedValue({...response,requests:[{...response.requests[0],status:SecurityAskStatus.APPROVED}]});
    await queue.load('vm-a');await queue.decide('ask-a',SecurityAskDecision.DENY);
    expect(resolve).toHaveBeenCalledTimes(1);
  });
  it('ignores stale responses after switching sessions or closing the panel',async()=>{
    let finish!:(value:SecurityAskListResponse)=>void;
    const queue=new ApprovalQueue(()=>new Promise(done=>{finish=done;}),vi.fn(),()=>{});
    const loading=queue.load('vm-a');queue.dispose();finish(response);await loading;
    expect(queue.state.response).toBeNull();
  });
  it('does not let an old decision reload a reopened panel for the same VM', async () => {
    const replacement = {...response, generation:'replacement-generation'};
    const read = vi.fn().mockResolvedValueOnce(response).mockResolvedValue(replacement);
    let finish!: () => void;
    const resolve = vi.fn(() => new Promise<void>(done => { finish = done; }));
    const queue = new ApprovalQueue(read, resolve, () => {}, () => 100);
    await queue.load('vm-a');
    const decision = queue.decide('ask-a', SecurityAskDecision.APPROVE);
    queue.dispose();
    await queue.load('vm-a');
    finish();
    await decision;
    expect(read).toHaveBeenCalledTimes(2);
    expect(resolve).toHaveBeenCalledExactlyOnceWith('vm-a', 'ask-a', 'generation-a', SecurityAskDecision.APPROVE);
    expect(queue.state.response).toEqual(replacement);
  });
  it('rejects a different VM response and keys notices by VM, generation and ask',async()=>{
    const queue=new ApprovalQueue(async()=>response,vi.fn(),()=>{});
    await queue.load('other-vm');expect(queue.state.response).toBeNull();expect(queue.state.error).toBeTruthy();
    expect(approvalKey(response,'ask-a')).not.toBe(approvalKey({...response,generation:'new'},'ask-a'));
    expect(approvalKey(response,'ask-a')).not.toBe(approvalKey({...response,vm_id:'new'},'ask-a'));
  });
});

it('rechecks a queue changed during a read and publishes only the newer result', async () => {
  const settled = {...response, pending_count:0};
  let finish!: (list: SecurityAskListResponse) => void;
  const read = vi.fn().mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValue(settled);
  const publish = vi.fn();
  const refresh = new ApprovalRefresh(read, publish, () => false);
  const first = refresh.enqueue(['vm-a']);
  await refresh.enqueue(['vm-a']); // A scheduled refresh can share the in-flight read.
  await refresh.enqueue(['vm-a'], true);
  finish(response);
  await first;
  expect(read.mock.calls).toEqual([['vm-a'], ['vm-a']]);
  expect(publish).toHaveBeenCalledExactlyOnceWith('vm-a', settled);
});

it('keeps notifications queued during a decision and cancels publication on disposal', async () => {
  let resolving = true;
  let finish!: (list: SecurityAskListResponse) => void;
  const read = vi.fn(() => new Promise<SecurityAskListResponse>(resolve => { finish = resolve; }));
  const publish = vi.fn();
  const refresh = new ApprovalRefresh(read, publish, () => resolving);
  await refresh.enqueue(['vm-a', 'vm-a']);
  expect(read).not.toHaveBeenCalled();
  resolving = false;
  const running = refresh.enqueue([]);
  expect(read).toHaveBeenCalledExactlyOnceWith('vm-a');
  refresh.dispose();
  finish(response);
  await running;
  expect(publish).not.toHaveBeenCalled();
});

it('continues to read other sessions while one session is resolving', async () => {
  const other = {...response, vm_id:'vm-b'};
  const read = vi.fn().mockResolvedValue(other);
  const publish = vi.fn();
  const refresh = new ApprovalRefresh(read, publish, id => id === 'vm-a');
  await refresh.enqueue(['vm-a', 'vm-b']);
  expect(read).toHaveBeenCalledExactlyOnceWith('vm-b');
  expect(publish).toHaveBeenCalledExactlyOnceWith('vm-b', other);
});

it('announces a pending request once per VM and generation and skips expired receipts',()=>{
  const notices=new ApprovalNotices();
  expect(notices.observe(response,100)).toEqual(['ask-a']);
  expect(notices.observe(response,100)).toEqual([]);
  expect(notices.observe({...response,generation:'new'},100)).toEqual(['ask-a']);
  expect(notices.observe({...response,vm_id:'other'},10001)).toEqual([]);
});

it('renders escaped bounded action facts, explicit decisions and resolved states',()=>{
  const pending={...response,requests:[{...response.requests[0],summary:'<script>unsafe</script>',expires_at_unix_ms:Date.now()+60000}]};
  const props={state:{response:pending,loading:false,resolving:null,error:null},vmName:'Session A',onclose:vi.fn(),onretry:vi.fn(),ondecide:vi.fn()};
  const html=render(Panel,{props}).body;
  expect(html).toContain('Approve once');expect(html).toContain('Deny');
  expect(html).toContain('&lt;script>unsafe&lt;/script>');expect(html).not.toContain('<script>unsafe');
  const resolved=render(Panel,{props:{...props,state:{...props.state,response:{...pending,requests:[{...pending.requests[0],status:SecurityAskStatus.APPROVED}]}}}}).body;
  expect(resolved).toContain('approved');expect(resolved).not.toContain('Approve once');
  for(const status of [SecurityAskStatus.DENIED,SecurityAskStatus.EXPIRED,SecurityAskStatus.CANCELLED]) {
    const result=render(Panel,{props:{...props,state:{...props.state,response:{...pending,requests:[{...pending.requests[0],status}]}}}}).body;
    expect(result).toContain(status===SecurityAskStatus.DENIED?'text-destructive-text':'text-muted-foreground-1');
    expect(result).not.toContain('Approve once');
  }
});
