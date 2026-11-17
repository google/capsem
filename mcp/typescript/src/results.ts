import {HttpError, NetworkError} from '@capsem/sdk';
import type {CallToolResult} from '@modelcontextprotocol/sdk/types.js';

type Structured = Record<string, unknown>;

/** Set to any value to write gateway failure bodies to stderr. */
export const DIAGNOSTICS_ENV = 'CAPSEM_MCP_DEBUG';

export function success(value: Structured): CallToolResult {
  // The SDK transport serializes the complete structured result once. A
  // second JSON text mirror or size-dependent summary adds no tool content.
  return {
    content: [],
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
    // stdout is the protocol channel and stderr stays quiet in normal
    // operation (an expected denial is an ordinary tool error), so the body
    // is written only when the operator asks for diagnostics. It can quote
    // credentials, which is also why it never reaches the tool result.
    if (process.env[DIAGNOSTICS_ENV]) {
      process.stderr.write(`capsem-mcp: gateway HTTP ${error.status}: ${error.body}\n`);
    }
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
