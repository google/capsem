import {defineConfig} from 'vitest/config';

export default defineConfig({
  test: {
    fileParallelism: false,
    coverage: {
      provider: 'v8', include: ['src/**/*.ts'],
      reporter: ['text', 'lcov'], reportsDirectory: '../../cache/target/coverage/mcp-typescript',
      thresholds: {lines: 90},
    },
  },
});
