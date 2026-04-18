import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import tailwindcss from '@tailwindcss/vite';
import mermaid from 'astro-mermaid';

export default defineConfig({
  site: 'https://docs.capsem.org',
  integrations: [
    starlight({
      title: 'Capsem',
      description: 'Sandbox AI agents in air-gapped Linux VMs on macOS.',
      logo: {
        src: './src/assets/logo.png',
      },
      favicon: '/favicon.svg',
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/google/capsem',
        },
      ],
      editLink: {
        baseUrl: 'https://github.com/google/capsem/edit/main/docs/',
      },
      lastUpdated: true,
      customCss: ['./src/styles/custom.css'],
      sidebar: [
        { slug: 'getting-started' },
        {
          label: 'Usage',
          autogenerate: { directory: 'usage' },
        },
        {
          label: 'Architecture',
          autogenerate: { directory: 'architecture' },
        },
        {
          label: 'Security',
          autogenerate: { directory: 'security' },
        },
        {
          label: 'Benchmarks',
          autogenerate: { directory: 'benchmarks' },
        },
        {
          label: 'Debugging',
          autogenerate: { directory: 'debugging' },
        },
        {
          label: 'Development',
          autogenerate: { directory: 'development' },
        },
        {
          label: 'Releases',
          collapsed: true,
          autogenerate: { directory: 'releases' },
        },
      ],
    }),
    mermaid(),
  ],
  vite: {
    plugins: [tailwindcss()],
  },
});
