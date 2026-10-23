import { invoke } from '@tauri-apps/api/core';
import { parseParentMessage } from './terminal/postmessage';

let nextRequestId = 0;

function readTokenFromParent(): Promise<string | null> {
  return new Promise(resolve => {
    const requestId = nextRequestId++;
    const parent = window.parent;
    const origin = window.location.origin;
    const finish = (token: string | null) => {
      clearTimeout(timeout);
      window.removeEventListener('message', onMessage);
      resolve(token);
    };
    const onMessage = (event: MessageEvent) => {
      if (event.source !== parent || event.origin !== origin) return;
      const msg = parseParentMessage(event.data);
      if (msg?.type === 'gateway-token' && msg.requestId === requestId) finish(msg.token);
    };
    const timeout = setTimeout(() => finish(null), 3000);
    window.addEventListener('message', onMessage);
    parent.postMessage({ type: 'gateway-token-request', requestId }, origin);
  });
}

/** Read the private gateway bearer through the desktop shell. */
export async function readGatewayToken(gatewayUrl: string): Promise<string | null> {
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    const token = await invoke<string>('get_gateway_token');
    return typeof token === 'string' && token.length === 64 ? token : null;
  }

  if (typeof window !== 'undefined' && window.parent && window.parent !== window) {
    return readTokenFromParent();
  }

  // Hermetic Vitest fixtures still exercise the API client with a mocked
  // transport. This branch is removed from production bundles.
  if (import.meta.env.MODE === 'test') {
    const response = await fetch(`${gatewayUrl}/token`);
    if (!response.ok) return null;
    const body: { token?: unknown } = await response.json();
    return typeof body.token === 'string' ? body.token : null;
  }
  return null;
}
