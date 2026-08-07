import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import tailwindcss from '@tailwindcss/vite';
import mermaid from 'astro-mermaid';

export default defineConfig({
  site: 'https://docs.capsem.org',
  integrations: [
    starlight({
      title: 'Capsem',
      description: 'The fastest way to ship with AI securely.',
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
          items: [{ autogenerate: { directory: 'usage' } }],
        },
        {
          label: 'Architecture',
          items: [{ autogenerate: { directory: 'architecture' } }],
        },
        {
          label: 'Security',
          items: [{ autogenerate: { directory: 'security' } }],
        },
        {
          label: 'Benchmarks',
          items: [{ autogenerate: { directory: 'benchmarks' } }],
        },
        {
          label: 'Debugging',
          items: [{ autogenerate: { directory: 'debugging' } }],
        },
        {
          label: 'Gotchas / FAQ',
          items: [{ autogenerate: { directory: 'gotchas' } }],
        },
        {
          label: 'Development',
          items: [{ autogenerate: { directory: 'development' } }],
        },
        {
          label: 'Releases',
          collapsed: true,
          items: [{ autogenerate: { directory: 'releases' } }],
        },
      ],
    }),
    mermaid(),
  ],
  vite: {
    plugins: [tailwindcss()],
  },
});
