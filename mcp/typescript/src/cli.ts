#!/usr/bin/env node

import {StdioServerTransport} from '@modelcontextprotocol/sdk/server/stdio.js';
import {parseConfig} from './config.js';
import {createServer} from './server.js';

async function main(): Promise<void> {
  const server = createServer(parseConfig(process.argv.slice(2)));
  await server.connect(new StdioServerTransport());
}

main().catch((error: unknown) => {
  // stdout is reserved for MCP frames. Configuration errors are TypeErrors
  // that name options but never their values; anything else stays generic.
  const reason = error instanceof TypeError ? error.message : 'failed to start';
  process.stderr.write(`capsem-mcp: ${reason}\n`);
  process.exitCode = 1;
});
