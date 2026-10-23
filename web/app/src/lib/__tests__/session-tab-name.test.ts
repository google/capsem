import { beforeEach, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
vi.mock('../api', () => ({getContainerStatus: vi.fn()}));
import TabBar from '../components/shell/TabBar.svelte';
import { tabStore } from '../stores/tabs.svelte';
import { vmStore } from '../stores/vms.svelte';
import type { VmSummary } from '../types/gateway';

beforeEach(() => {
  tabStore.tabs = [];
  tabStore.add();
  vmStore.vms = [];
});

it('renders late-arriving and renamed session names without reopening a deep-linked tab', () => {
  tabStore.openVM('session-uuid', 'session-uuid', 'ui');
  const id = tabStore.activeId;
  expect(render(TabBar).body).toContain('session-uuid');
  vmStore.vms = [{id:'session-uuid',name:'Antigravity IDE'} as VmSummary];
  expect(render(TabBar).body).toContain('Antigravity IDE');
  expect(render(TabBar).body).not.toContain('session-uuid');
  vmStore.vms = [{id:'session-uuid',name:'My editor'} as VmSummary];
  expect(render(TabBar).body).toContain('My editor');
  expect(tabStore.activeId).toBe(id);
  vmStore.vms = [];
  expect(render(TabBar).body).toContain('session-uuid');
  expect(render(TabBar).body).toContain('Dashboard');
});
