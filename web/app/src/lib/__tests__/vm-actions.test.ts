import { VmAction, VmLifecycleState } from '@capsem/sdk';
import { describe, expect, it } from 'vitest';

import { canOpenSession, hasVmAction } from '../vm-actions';
import type { VmSummary } from '../types/gateway';

function vm(status: VmSummary['status'], available_actions: VmSummary['available_actions']): VmSummary {
  return {
    id: `${status.toLowerCase()}-vm`,
    name: null,
    status,
    persistent: true,
    profile_id: 'code',
    can_resume: false,
    available_actions,
  };
}

describe('vm-actions', () => {
  it('uses backend available_actions instead of status guessing', () => {
    const incompatible = vm(VmLifecycleState.INCOMPATIBLE, [VmAction.DELETE]);
    const defunct = vm(VmLifecycleState.DEFUNCT, [VmAction.DELETE]);
    const stopped = vm(VmLifecycleState.STOPPED, [VmAction.START, VmAction.FORK, VmAction.DELETE]);

    expect(hasVmAction(incompatible, VmAction.START)).toBe(false);
    expect(hasVmAction(incompatible, VmAction.FORK)).toBe(false);
    expect(hasVmAction(incompatible, VmAction.DELETE)).toBe(true);
    expect(canOpenSession(incompatible)).toBe(false);

    expect(hasVmAction(defunct, VmAction.RESUME)).toBe(false);
    expect(hasVmAction(defunct, VmAction.FORK)).toBe(false);
    expect(hasVmAction(defunct, VmAction.DELETE)).toBe(true);
    expect(canOpenSession(defunct)).toBe(false);

    expect(hasVmAction(stopped, VmAction.START)).toBe(true);
    expect(canOpenSession(stopped)).toBe(true);
  });

  it('caps terminal sessions to delete-only even if stale actions leak through', () => {
    const incompatible = vm(VmLifecycleState.INCOMPATIBLE, [VmAction.START, VmAction.FORK, VmAction.DELETE]);
    const defunct = vm(VmLifecycleState.DEFUNCT, [VmAction.RESUME, VmAction.FORK, VmAction.DELETE]);

    expect(hasVmAction(incompatible, VmAction.START)).toBe(false);
    expect(hasVmAction(incompatible, VmAction.FORK)).toBe(false);
    expect(hasVmAction(incompatible, VmAction.DELETE)).toBe(true);
    expect(canOpenSession(incompatible)).toBe(false);

    expect(hasVmAction(defunct, VmAction.RESUME)).toBe(false);
    expect(hasVmAction(defunct, VmAction.FORK)).toBe(false);
    expect(hasVmAction(defunct, VmAction.DELETE)).toBe(true);
    expect(canOpenSession(defunct)).toBe(false);
  });
});
