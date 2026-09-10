import {defineConfig} from 'vitest/config';

export default defineConfig({
  test: {
    include: ['tests/**/*.test.ts'],
    coverage: {
      provider: 'v8', include: ['src/**/*.ts'], exclude: [],
      reporter: ['text', 'lcov'], reportsDirectory: '../../cache/target/coverage/typescript-sdk',
      thresholds: {lines: 90, branches: 90, functions: 90, statements: 90},
    },
  },
});
