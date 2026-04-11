import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  test: {
    include: ['src/lib/__tests__/**/*.test.ts', 'src/lib/models/__tests__/**/*.test.ts'],
  },
});
