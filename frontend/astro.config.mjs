import { defineConfig } from 'astro/config';
import svelte from '@astrojs/svelte';
import tailwindcss from '@tailwindcss/vite';
import releaseNotes from './plugins/vite-plugin-release-notes';

export default defineConfig({
  output: 'static',
  server: { port: 5173 },
  integrations: [svelte()],
  vite: {
    envPrefix: ['VITE_', 'TAURI_'],
    define: {
      __BUILD_TS__: JSON.stringify(new Date().toISOString().replace('T', ' ').slice(0, 19)),
    },
    plugins: [tailwindcss(), releaseNotes()],
  },
});
