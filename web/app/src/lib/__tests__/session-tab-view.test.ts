import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ContainerState, ContainerSurfaceKind, type ContainerStatusResponse } from '@capsem/sdk';
vi.mock('../api', () => ({getContainerStatus: vi.fn()}));
const {getContainerStatus}=await import('../api');
const {tabStore}=await import('../stores/tabs.svelte');
const graphical:ContainerStatusResponse={state:ContainerState.RUNNING,surface:{kind:ContainerSurfaceKind.XPRA,port:14500},image:'desktop'};
beforeEach(()=>{
  tabStore.tabs=[];
  tabStore.add();
  vi.mocked(getContainerStatus).mockReset().mockResolvedValue(null);
});
describe('session tab presentation',()=>{
  it('opens known graphical images directly in UI without starting a terminal',()=>{
    tabStore.openVM('gui','Desktop','ui');
    expect(tabStore.active?.view).toBe('ui');
    expect(tabStore.active?.terminalOpened).toBeFalsy();
    expect(getContainerStatus).not.toHaveBeenCalled();
  });
  it('discovers the presentation when reopening an existing session',async()=>{
    vi.mocked(getContainerStatus).mockResolvedValue(graphical);
    tabStore.openVM('gui','Desktop');
    expect(tabStore.active?.presentationPending).toBe(true);
    await vi.waitFor(()=>expect(tabStore.active?.view).toBe('ui'));
    expect(tabStore.active?.terminalOpened).toBeFalsy();
  });
  it('preserves an explicit view change while presentation is being resolved',async()=>{
    let resolve!:(value:ContainerStatusResponse)=>void;
    vi.mocked(getContainerStatus).mockReturnValue(new Promise(done=>{resolve=done;}));
    tabStore.openVM('gui','Desktop');
    const tab=tabStore.active!;
    tabStore.updateView(tab.id,'stats');
    resolve(graphical);
    await vi.waitFor(()=>expect(tab.surface).toBe(true));
    expect(tab.view).toBe('stats');
  });
  it('reuses the same session tab while Stats is selected',()=>{
    tabStore.openVM('shell','Shell','terminal');
    const id=tabStore.activeId;
    tabStore.updateView(id,'stats');
    tabStore.openVM('shell','Shell');
    expect(tabStore.activeId).toBe(id);
    expect(tabStore.tabs.filter(tab=>tab.vmId==='shell')).toHaveLength(1);
    expect(tabStore.active?.view).toBe('stats');
  });
});
