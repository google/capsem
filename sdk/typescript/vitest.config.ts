import {defineConfig} from 'vitest/config';
import {fileURLToPath} from 'node:url';

export default defineConfig({
  resolve: {alias: {'#local': fileURLToPath(new URL('./src/local-node.ts', import.meta.url))}},
  test: {
    include: ['tests/**/*.test.ts'],
    fileParallelism: false,
    coverage: {
      provider: 'v8', include: ['src/**/*.ts'], exclude: [],
      reporter: ['text', 'lcov'], reportsDirectory: '../../cache/target/coverage/typescript-sdk',
      thresholds: {lines: 90, branches: 90, functions: 90, statements: 90},
    },
  },
});
