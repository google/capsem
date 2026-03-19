import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'path';

export default defineConfig({
  plugins: [svelte({ hot: false })],
  resolve: {
    alias: {
      '@tauri-apps/api/core': resolve(__dirname, 'src/lib/__mocks__/tauri.ts'),
      '@tauri-apps/api/event': resolve(__dirname, 'src/lib/__mocks__/tauri.ts'),
    },
    conditions: ['browser'],
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
  },
});
