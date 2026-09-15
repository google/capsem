import {defineConfig} from 'vitest/config';

export default defineConfig({
  test: {
    fileParallelism: false,
    coverage: {provider: 'v8', reporter: ['text'], thresholds: {lines: 90}},
  },
});
