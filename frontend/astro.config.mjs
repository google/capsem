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
      // Every Shiki grammar and theme is a dynamic-import chunk fetched
      // on first use (see frontend/src/lib/shiki.ts). The startup graph
      // has no chunk >200 KB. The sole exception is cpp (~620 KB), which
      // is the inherent size of the C++ TextMate grammar upstream and
      // only fetched when the user opens a .cpp/.hpp/.cc/.cxx file. Any
      // other chunk crossing 700 KB is a real regression -- don't raise
      // this further without naming which new chunk hit the ceiling.
      chunkSizeWarningLimit: 700,
    },
  },
});
