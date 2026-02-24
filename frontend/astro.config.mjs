import { defineConfig } from 'astro/config';

export default defineConfig({
  output: 'static',
  server: { port: 5173 },
  vite: {
    // Let Tauri handle env vars
    envPrefix: ['VITE_', 'TAURI_'],
  },
});
