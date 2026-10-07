import { describe, expect, it } from 'vitest';
import { ApiError } from '../gateway-sdk';
import { sessionActionError } from '../models/session-action-error';

describe('session action error presentation', () => {
  it('presents the service explanation without raw wire JSON', () => {
    expect(sessionActionError(new ApiError(403, '{"error":"catalog image refused by policy"}'))).toBe('catalog image refused by policy');
  });
  it('keeps status for unreadable replies and bounds terminal control characters', () => {
    expect(sessionActionError(new ApiError(504, '<html>internal proxy</html>'))).toBe('Gateway action failed (504)');
    expect(sessionActionError(new ApiError(409, JSON.stringify({ error: '\u001b[1mcheckpoint\n' + 'x'.repeat(2048) })))).toHaveLength(512);
    expect(sessionActionError(new Error('creation failed'))).toBe('creation failed');
    expect(sessionActionError(null)).toBe('Failed to create session');
  });
});
