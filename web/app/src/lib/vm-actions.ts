import { VmAction, VmLifecycleState, type VmSummary } from '@capsem/sdk';

function isTerminalSession(vm: Pick<VmSummary, 'status'>): boolean {
  return vm.status === VmLifecycleState.DEFUNCT || vm.status === VmLifecycleState.INCOMPATIBLE;
}

export function hasVmAction(vm: Pick<VmSummary, 'status' | 'available_actions'>, action: VmAction): boolean {
  if (isTerminalSession(vm) && action !== VmAction.DELETE) return false;
  return vm.available_actions.includes(action);
}

export function canOpenSession(vm: Pick<VmSummary, 'status' | 'available_actions'>): boolean {
  return !isTerminalSession(vm);
}

export function startLabel(vm: Pick<VmSummary, 'status'>): string {
  return vm.status === VmLifecycleState.SUSPENDED ? 'Resume' : 'Start';
}

export function startAction(vm: Pick<VmSummary, 'status'>): VmAction {
  return vm.status === VmLifecycleState.SUSPENDED ? VmAction.RESUME : VmAction.START;
}
