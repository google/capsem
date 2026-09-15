import { HttpError } from '@capsem/sdk';
import { Transport } from '@capsem/sdk/transport';

export class ApiError extends Error {
  constructor(public status: number, public body: string) {
    super(`API error ${status}: ${body}`);
    this.name = 'ApiError';
  }
}

export function isAuthRefreshStatus(status: number): boolean {
  return status === 401 || status === 429;
}

interface Connection {
  url(): string;
  token(): string | null;
  refreshToken(): Promise<boolean>;
}

/** The UI owns token refresh; the SDK owns authenticated HTTP and validation. */
export class GatewaySdk {
  constructor(private readonly connection: Connection) {}

  async call<T>(operation: (transport: Transport) => Promise<T>, retryAuth = true): Promise<T> {
    const token = this.connection.token();
    if (!token) throw new Error('Gateway not connected');
    const transport = new Transport(this.connection.url(), token);
    try {
      return await operation(transport);
    } catch (error) {
      if (!(error instanceof HttpError)) throw error;
      if (retryAuth && isAuthRefreshStatus(error.status) && await this.connection.refreshToken()) {
        return this.call(operation, false);
      }
      throw new ApiError(error.status, error.body);
    } finally {
      transport.close();
    }
  }
}
