import { invoke } from '@tauri-apps/api/core';
import { parseParentMessage } from './terminal/postmessage';
import { parseGatewayConnection, type GatewayConnection } from './gateway-connection';

let nextRequestId = 0;

function readConnectionFromParent(): Promise<GatewayConnection | null> {
  return new Promise(resolve => {
    const requestId = nextRequestId++;
    const parent = window.parent;
    const origin = window.location.origin;
    const finish = (connection: GatewayConnection | null) => {
      clearTimeout(timeout);
      window.removeEventListener('message', onMessage);
      resolve(connection);
    };
    const onMessage = (event: MessageEvent) => {
      if (event.source !== parent || event.origin !== origin) return;
      const msg = parseParentMessage(event.data);
      if (msg?.type === 'gateway-token' && msg.requestId === requestId) finish(parseGatewayConnection(msg));
    };
    const timeout = setTimeout(() => finish(null), 3000);
    window.addEventListener('message', onMessage);
    parent.postMessage({ type: 'gateway-token-request', requestId }, origin);
  });
}

export function needsGatewayConnection(): boolean {
  return typeof window !== 'undefined' && ('__TAURI_INTERNALS__' in window || Boolean(window.parent && window.parent !== window));
}

/** Read matching origin and authority through the desktop or its trusted parent. */
export async function readGatewayConnection(gatewayUrl: string): Promise<GatewayConnection | null> {
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    return parseGatewayConnection(await invoke<unknown>('get_gateway_connection'));
  }

  if (typeof window !== 'undefined' && window.parent && window.parent !== window) {
    return readConnectionFromParent();
  }

  // Hermetic Vitest fixtures still exercise the API client with a mocked
  // transport. This branch is removed from production bundles.
  if (import.meta.env.MODE === 'test') {
    const response = await fetch(`${gatewayUrl}/token`);
    if (!response.ok) return null;
    const body: { token?: unknown } = await response.json();
    return typeof body.token === 'string' ? { gatewayUrl, token: body.token } : null;
  }
  return null;
}

/** A refresh may rotate authority, but cannot silently change its destination. */
export async function readGatewayToken(gatewayUrl: string): Promise<string | null> {
  const connection = await readGatewayConnection(gatewayUrl);
  return connection?.gatewayUrl === gatewayUrl ? connection.token : null;
}
