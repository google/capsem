import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'path';
import releaseNotes from './plugins/vite-plugin-release-notes';

export default defineConfig({
  plugins: [svelte({ hot: false }), releaseNotes()],
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
    coverage: {
      provider: 'v8',
      include: ['src/**/*.{ts,svelte}'],
      exclude: ['src/**/*.test.ts', 'src/**/__mocks__/**'],
      reporter: ['text', 'json'],
      reportsDirectory: '../coverage/frontend',
    },
  },
});
