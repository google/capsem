import { beforeEach, expect, it, vi } from 'vitest';
import { VmAction, VmLifecycleState, type VmSummary } from '@capsem/sdk';

const api = vi.hoisted(() => ({ getStatus: vi.fn(), stopVm: vi.fn(), startVm: vi.fn(), resumeVm: vi.fn(), suspendVm: vi.fn(), deleteVm: vi.fn(), forkVm: vi.fn() }));
vi.mock('../api', () => api);
const { vmStore } = await import('../stores/vms.svelte');

function session(status: VmLifecycleState, actions: VmAction[]): VmSummary {
  return { id: 'session-id', name: 'Workspace', status, available_actions: actions, can_resume: true, persistent: true };
}

beforeEach(() => {
  vi.resetAllMocks();
  vmStore.destroy();
  vmStore.vms = [];
  vmStore.error = null;
  vmStore.acting = false;
  vmStore.serviceStatus = 'running';
  api.getStatus.mockResolvedValue({ service: 'running', vms: [], resource_summary: null });
});

it('carries a catalog launch choice into the create dialog and clears it on close', () => {
  vmStore.openCreateModal('claude-code');
  expect(vmStore.showCreateModal).toBe(true);
  expect(vmStore.createImageName).toBe('claude-code');
  vmStore.closeCreateModal();
  expect(vmStore.createImageName).toBe('');
  vmStore.openCreateModal();
  expect(vmStore.createImageName).toBe('');
});

it('dispatches the currently advertised recovery operation', async () => {
  vmStore.vms = [session(VmLifecycleState.STOPPED, [VmAction.START])];
  await vmStore.resume('session-id');
  expect(api.startVm).toHaveBeenCalledWith('session-id');
  expect(api.resumeVm).not.toHaveBeenCalled();
  vi.clearAllMocks();
  vmStore.vms = [session(VmLifecycleState.SUSPENDED, [VmAction.RESUME])];
  await vmStore.resume('session-id');
  expect(api.resumeVm).toHaveBeenCalledWith('session-id');
  expect(api.startVm).not.toHaveBeenCalled();
});

it('rejects revoked operations before dispatching an API request', async () => {
  vmStore.vms = [session(VmLifecycleState.RUNNING, [])];
  for (const operation of [() => vmStore.stop('session-id'), () => vmStore.suspend('session-id'), () => vmStore.delete('session-id'), () => vmStore.fork('session-id', { name: 'fork' }), () => vmStore.resume('session-id')]) {
    await expect(operation()).rejects.toThrow('no longer available');
  }
  for (const operation of [api.stopVm, api.suspendVm, api.deleteVm, api.forkVm, api.startVm, api.resumeVm]) expect(operation).not.toHaveBeenCalled();
  expect(api.getStatus).not.toHaveBeenCalled();
  expect(vmStore.acting).toBe(false);
});

it('leaves refused deletions and their original error intact', async () => {
  const vm = session(VmLifecycleState.RUNNING, [VmAction.DELETE]);
  vmStore.vms = [vm];
  const error = new Error('managed session cannot be deleted yet');
  api.deleteVm.mockRejectedValue(error);
  await expect(vmStore.delete(vm.id)).rejects.toBe(error);
  expect(vmStore.vms).toEqual([vm]);
  expect(vmStore.acting).toBe(false);
});

it('purges exactly broken sessions, including incompatible persistent sessions', async () => {
  const broken = [
    { ...session(VmLifecycleState.INCOMPATIBLE, [VmAction.DELETE]), id: 'legacy' },
    { ...session(VmLifecycleState.DEFUNCT, [VmAction.DELETE]), id: 'defunct' },
  ];
  const healthy = { ...session(VmLifecycleState.STOPPED, [VmAction.DELETE]), id: 'healthy', persistent: false };
  vmStore.vms = [...broken, healthy];
  api.getStatus.mockImplementation(async () => ({ service: 'running', vms: vmStore.vms }));
  expect(await vmStore.deleteBroken()).toEqual(['legacy', 'defunct']);
  expect(api.deleteVm.mock.calls).toEqual([['legacy'], ['defunct']]);
  expect(vmStore.vms).toEqual([healthy]);
});

it('reports a refused broken-session deletion without hiding the remaining sessions', async () => {
  vmStore.vms = [session(VmLifecycleState.INCOMPATIBLE, [VmAction.DELETE])];
  api.deleteVm.mockRejectedValue(new Error('credential retirement pending'));
  await expect(vmStore.deleteBroken()).rejects.toThrow('credential retirement pending');
  expect(vmStore.vms).toHaveLength(1);
});

it('revokes actions when the session vanishes or status becomes unavailable', async () => {
  for (const condition of ['missing', 'offline', 'failed-poll']) {
    vmStore.vms = condition === 'missing' ? [] : [session(VmLifecycleState.RUNNING, [VmAction.STOP])];
    vmStore.serviceStatus = condition === 'offline' ? 'offline' : 'running';
    vmStore.error = condition === 'failed-poll' ? 'status unavailable' : null;
    await expect(vmStore.stop('session-id')).rejects.toThrow('no longer available');
  }
  expect(api.stopVm).not.toHaveBeenCalled();
});
