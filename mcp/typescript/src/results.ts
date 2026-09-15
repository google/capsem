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
  if (error instanceof HttpError) message = `Gateway returned HTTP ${error.status}`;
  else if (error instanceof NetworkError) message = 'Gateway connection failed';
  else if (error instanceof TypeError) message = error.message;
  else if (error instanceof Error && error.name === 'AbortError') message = 'Gateway request cancelled';
  return {content: [{type: 'text', text: message}], isError: true};
}

export async function toolCall<T extends object>(operation: () => Promise<T>): Promise<CallToolResult> {
  try {
    return success(await operation() as Structured);
  } catch (error) {
    return failure(error);
  }
}
