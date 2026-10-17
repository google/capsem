import { ApiError } from '../gateway-sdk';

export function sessionActionError(error: unknown): string {
  let message = error instanceof Error ? error.message : 'Failed to create session';
  if (error instanceof ApiError) {
    message = `Gateway action failed (${error.status})`;
    try {
      const detail: unknown = JSON.parse(error.body);
      if (detail && typeof detail === 'object' && 'error' in detail && typeof detail.error === 'string' && detail.error.trim()) message = detail.error;
    } catch { /* Unreadable gateway replies retain their status. */ }
  }
  return message.replace(/[\u0000-\u001f\u007f]/g, '').slice(0, 512);
}
