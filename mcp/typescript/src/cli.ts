#!/usr/bin/env node

import {StdioServerTransport} from '@modelcontextprotocol/sdk/server/stdio.js';
import {parseConfig} from './config.js';
import {createServer} from './server.js';

async function main(): Promise<void> {
  const server = createServer(parseConfig(process.argv.slice(2)));
  await server.connect(new StdioServerTransport());
}

main().catch(() => {
  // stdout is reserved for MCP frames. Never include token-bearing arguments.
  process.stderr.write('capsem-mcp: failed to start\n');
  process.exitCode = 1;
});
