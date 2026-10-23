/** Native authority is bound to one strict loopback origin. */
export interface GatewayConnection { gatewayUrl: string; token: string }

export function parseGatewayConnection(value: unknown): GatewayConnection | null {
  if (!value || typeof value !== 'object') return null;
  const { gatewayUrl, token } = value as Record<string, unknown>;
  if (typeof gatewayUrl !== 'string' || typeof token !== 'string' || !/^[a-zA-Z0-9]{64}$/.test(token)) return null;
  const match = /^http:\/\/(?:127\.0\.0\.1|localhost)(?::([0-9]{1,5}))?$/.exec(gatewayUrl);
  const port = match?.[1] === undefined ? 80 : Number(match[1]);
  return match && port > 0 && port <= 65535 ? { gatewayUrl, token } : null;
}
