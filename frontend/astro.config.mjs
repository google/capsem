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
    build: {
      // Startup chunks are all <200 KB after code-splitting. The one
      // exception is the cpp TextMate grammar (~620 KB) -- inherent to
      // Shiki upstream and loaded *only* on demand from ensureShikiLang()
      // when the user opens a .cpp/.hpp/.cc/.cxx/.rb file. Any other
      // chunk crossing 700 KB is a real regression worth investigating.
      chunkSizeWarningLimit: 700,
    },
  },
});
