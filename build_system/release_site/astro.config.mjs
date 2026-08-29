import { defineConfig } from 'astro/config';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  site: 'https://release.capsem.org',
  trailingSlash: 'always',
  vite: {
    plugins: [tailwindcss()],
  },
});
