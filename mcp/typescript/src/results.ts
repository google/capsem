import {HttpError, NetworkError} from '@capsem/sdk';
import type {CallToolResult} from '@modelcontextprotocol/sdk/types.js';

type Structured = Record<string, unknown>;

export function success(value: Structured): CallToolResult {
  return {
    content: [{type: 'text', text: JSON.stringify(value)}],
    structuredContent: value,
  };
}

export function failure(error: unknown): CallToolResult {
  let message = 'Gateway request failed';
  let kind = 'internal';
  let status: number | undefined;
  if (error instanceof HttpError) {
    message = `Gateway returned HTTP ${error.status}`;
    kind = 'http';
    status = error.status;
  } else if (error instanceof NetworkError) {
    message = 'Gateway connection failed';
    kind = 'network';
  } else if (error instanceof TypeError) {
    message = error.message;
    kind = 'invalid_input';
  } else if (error instanceof Error && error.name === 'AbortError') {
    message = 'Gateway request cancelled';
    kind = 'cancelled';
  } else if (error instanceof Error && error.name === 'TimeoutError') {
    message = 'Gateway request deadline exceeded';
    kind = 'timeout';
  }
  return {
    content: [{type: 'text', text: message}],
    structuredContent: {error: {kind, ...(status === undefined ? {} : {status})}},
    isError: true,
  };
}

export async function toolCall<T extends object>(operation: () => Promise<T>): Promise<CallToolResult> {
  try {
    return success(await operation() as Structured);
  } catch (error) {
    return failure(error);
  }
}
