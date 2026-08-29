import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    coverage: {
      include: ['scripts/**/*.mjs', 'src/**/*.ts'],
      provider: 'v8',
      reporter: ['text', 'lcov'],
      reportsDirectory: '../../target/coverage/distribution-site',
    },
    include: ['../tests/release_site/**/*.test.ts'],
  },
});
