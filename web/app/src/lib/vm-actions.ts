import type { VmAction, VmSummary } from './types/gateway';

function isTerminalSession(vm: Pick<VmSummary, 'status'>): boolean {
  return vm.status === 'Defunct' || vm.status === 'Incompatible';
}

export function hasVmAction(vm: Pick<VmSummary, 'status' | 'available_actions'>, action: VmAction): boolean {
  if (isTerminalSession(vm) && action !== 'delete') return false;
  return vm.available_actions.includes(action);
}

export function canOpenSession(vm: Pick<VmSummary, 'status' | 'available_actions'>): boolean {
  return !isTerminalSession(vm);
}

export function startLabel(vm: Pick<VmSummary, 'status'>): string {
  return vm.status === 'Suspended' ? 'Resume' : 'Start';
}

export function startAction(vm: Pick<VmSummary, 'status'>): VmAction {
  return vm.status === 'Suspended' ? 'resume' : 'start';
}
