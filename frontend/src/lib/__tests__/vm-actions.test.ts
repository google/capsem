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
    const incompatible = vm('Incompatible', ['delete']);
    const defunct = vm('Defunct', ['delete']);
    const stopped = vm('Stopped', ['start', 'fork', 'delete']);

    expect(hasVmAction(incompatible, 'start')).toBe(false);
    expect(hasVmAction(incompatible, 'fork')).toBe(false);
    expect(hasVmAction(incompatible, 'delete')).toBe(true);
    expect(canOpenSession(incompatible)).toBe(false);

    expect(hasVmAction(defunct, 'resume')).toBe(false);
    expect(hasVmAction(defunct, 'fork')).toBe(false);
    expect(hasVmAction(defunct, 'delete')).toBe(true);
    expect(canOpenSession(defunct)).toBe(false);

    expect(hasVmAction(stopped, 'start')).toBe(true);
    expect(canOpenSession(stopped)).toBe(true);
  });
});
