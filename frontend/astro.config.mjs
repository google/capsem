import { defineConfig } from 'astro/config';
import svelte from '@astrojs/svelte';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  output: 'static',
  server: { port: 5173 },
  integrations: [svelte()],
  vite: {
    envPrefix: ['VITE_', 'TAURI_'],
    plugins: [tailwindcss()],
  },
});
